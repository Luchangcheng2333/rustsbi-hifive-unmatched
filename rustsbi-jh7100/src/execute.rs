use crate::feature;
use crate::hsm::{pause, HsmCommand, U74Hsm};
use crate::runtime::{MachineTrap, Runtime, SupervisorContext};
use core::{
    ops::{Generator, GeneratorState},
    pin::Pin,
};
use riscv::register::scause::{Exception, Interrupt, Trap};
use riscv::register::*;

pub fn execute_supervisor(supervisor_mepc: usize, hart_id: usize, opaque: usize, hsm: U74Hsm) {
    let mut rt = Runtime::new_sbi_supervisor(supervisor_mepc, hart_id, opaque);
    hsm.record_current_start_finished();
    loop {
        match Pin::new(&mut rt).resume(()) {
            GeneratorState::Yielded(MachineTrap::SbiCall()) => {
                let ctx = rt.context_mut();
                let param = [ctx.a0, ctx.a1, ctx.a2, ctx.a3, ctx.a4, ctx.a5];
                let ans = rustsbi::ecall(ctx.a7, ctx.a6, param);
                if ans.error == 0x233 {
                    // hart non-retentive resume
                    if let Some(HsmCommand::Start(start_paddr, opaque)) = hsm.last_command() {
                        unsafe {
                            satp::write(0);
                            sstatus::clear_sie();
                        }
                        hsm.record_current_start_finished();
                        ctx.mstatus = mstatus::read(); // get from modified sstatus
                        ctx.a0 = hart_id;
                        ctx.a1 = opaque;
                        ctx.mepc = start_paddr;
                    }
                } else {
                    ctx.a0 = ans.error;
                    ctx.a1 = ans.value;
                    ctx.mepc = ctx.mepc.wrapping_add(4);
                }
            }
            GeneratorState::Yielded(MachineTrap::IllegalInstruction()) => {
                let ctx = rt.context_mut();
                // FIXME: get_vaddr_u32这个过程可能出错。
                let ins = unsafe { get_vaddr_u32(ctx.mepc) } as usize;
                if !emulate_illegal_instruction(ctx, ins) {
                    unsafe {
                        if feature::should_transfer_trap(ctx) {
                            feature::do_transfer_trap(
                                ctx,
                                Trap::Exception(Exception::IllegalInstruction),
                            )
                        } else {
                            fail_illegal_instruction(ctx, ins)
                        }
                    }
                }
            }
            GeneratorState::Yielded(MachineTrap::MachineTimer()) => unsafe {
                mip::set_stimer();
                mie::clear_mtimer();
            },
            GeneratorState::Yielded(MachineTrap::MachineSoft()) => match hsm.last_command() {
                Some(HsmCommand::Start(_start_paddr, _opaque)) => {
                    panic!("rustsbi-jh7100: illegal state")
                }
                Some(HsmCommand::Stop) => {
                    // no hart stop command in JH7100, record stop state and pause
                    hsm.record_current_stop_finished();
                    pause();
                    if let Some(HsmCommand::Start(start_paddr, opaque)) = hsm.last_command() {
                        // Resuming from a non-retentive suspend state is relatively more involved and requires software
                        // to restore various hart registers and CSRs for all privilege modes.
                        // Upon resuming from non-retentive suspend state, the hart will jump to supervisor-mode at address
                        // specified by `resume_addr` with specific registers values described in the table below:
                        //
                        // | Register Name | Register Value
                        // |:--------------|:--------------
                        // | `satp`        | 0
                        // | `sstatus.SIE` | 0
                        // | a0            | hartid
                        // | a1            | `opaque` parameter
                        unsafe {
                            satp::write(0);
                            sstatus::clear_sie();
                        }
                        hsm.record_current_start_finished();
                        let ctx = rt.context_mut();
                        ctx.mstatus = mstatus::read(); // get from modified sstatus
                        ctx.a0 = hart_id;
                        ctx.a1 = opaque;
                        ctx.mepc = start_paddr;
                    }
                }
                None => unsafe {
                    // machine software interrupt but no HSM commands - delegate to S mode;
                    let ctx = rt.context_mut();
                    let clint = crate::peripheral::Clint::new(0x2000000 as *mut u8);
                    clint.clear_soft(hart_id); // Clear IPI
                    if feature::should_transfer_trap(ctx) {
                        feature::do_transfer_trap(ctx, Trap::Interrupt(Interrupt::SupervisorSoft))
                    } else {
                        panic!("rustsbi-jh7100: machine soft interrupt with no hart state monitor command")
                    }
                },
            },
            GeneratorState::Complete(()) => break,
        }
    }
}

#[inline]
unsafe fn get_vaddr_u32(vaddr: usize) -> u32 {
    let mut ans: u32;
    core::arch::asm!("
        li      {tmp}, (1 << 17)
        csrrs   {tmp}, mstatus, {tmp}
        lwu     {ans}, 0({vaddr})
        csrw    mstatus, {tmp}
        ",
        tmp = out(reg) _,
        vaddr = in(reg) vaddr,
        ans = lateout(reg) ans
    );
    ans
}

fn emulate_illegal_instruction(ctx: &mut SupervisorContext, ins: usize) -> bool {
    if feature::emulate_rdtime(ctx, ins) {
        return true;
    }
    false
}

// 真·非法指令异常，是M层出现的
fn fail_illegal_instruction(ctx: &mut SupervisorContext, ins: usize) -> ! {
    #[cfg(target_pointer_width = "64")]
    panic!("invalid instruction from machine level, mepc: {:016x?}, instruction: {:016x?}, context: {:016x?}", ctx.mepc, ins, ctx);
    #[cfg(target_pointer_width = "32")]
    panic!("invalid instruction from machine level, mepc: {:08x?}, instruction: {:08x?}, context: {:08x?}", ctx.mepc, ins, ctx);
}

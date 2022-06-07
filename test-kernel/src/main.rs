#![feature(naked_functions, asm_sym, asm_const)]
#![feature(default_alloc_error_handler)]
#![no_std]
#![no_main]

mod console;
mod mm;
mod sbi;
mod util;

use riscv::register::{
    scause::{self, Exception, Trap},
    sepc,
    stvec::{self, TrapMode},
};

pub extern "C" fn rust_main(hartid: usize, dtb_pa: usize) -> ! {
    if hartid == 0 {
        // initialization
        mm::init_heap();
    }
    if hartid == 0 {
        println!(
            "<< Test-kernel: Hart id = {}, DTB physical address = {:#x}",
            hartid, dtb_pa
        );
        test_base_extension();
        test_sbi_ins_emulation();
        unsafe { stvec::write(start_trap as usize, TrapMode::Direct) };
        println!(">> Test-kernel: Trigger illegal exception");
        unsafe { core::arch::asm!("csrw mcycle, x0") }; // mcycle cannot be written, this is always a 4-byte illegal instruction
    }
    if hartid == 0 {
        for i in 0..2 {
            let sbi_ret = sbi::hart_get_status(i);
            println!(">> Hart {} state return value: {:?}", i, sbi_ret);
        }
    } else {
        let sbi_ret = sbi::hart_suspend(0x00000000, 0, 0);
        println!(
            ">> Start test for hart {}, retentive suspend return value {:?}",
            hartid, sbi_ret
        );
    }
    if hartid == 0 {
        println!(
            "<< Test-kernel: test for hart {} success, wake another hart",
            hartid
        );
        let bv: usize = 0b10;
        let sbi_ret = sbi::send_ipi(&bv as *const _ as usize, hartid); // wake hartid + 1
        println!(">> Wake hart 1, sbi return value {:?}", sbi_ret);
        loop {} // wait for machine shutdown
    } else {
        // hartid == 2 || hartid == 3
        unreachable!()
    }
}

fn test_base_extension() {
    println!(">> Test-kernel: Testing base extension");
    let base_version = sbi::probe_extension(sbi::EXTENSION_BASE);
    if base_version == 0 {
        println!("!! Test-kernel: no base extension probed; SBI call returned value '0'");
        println!(
            "!! Test-kernel: This SBI implementation may only have legacy extension implemented"
        );
        println!("!! Test-kernel: SBI test FAILED due to no base extension found");
        sbi::shutdown()
    }
    println!("<< Test-kernel: Base extension version: {:x}", base_version);
    println!(
        "<< Test-kernel: SBI specification version: {:x}",
        sbi::get_spec_version()
    );
    println!(
        "<< Test-kernel: SBI implementation Id: {:x}",
        sbi::get_sbi_impl_id()
    );
    println!(
        "<< Test-kernel: SBI implementation version: {:x}",
        sbi::get_sbi_impl_version()
    );
    println!(
        "<< Test-kernel: Device mvendorid: {:x}",
        sbi::get_mvendorid()
    );
    println!("<< Test-kernel: Device marchid: {:x}", sbi::get_marchid());
    println!("<< Test-kernel: Device mimpid: {:x}", sbi::get_mimpid());
}

fn test_sbi_ins_emulation() {
    println!(">> Test-kernel: Testing SBI instruction emulation");
    let time_start = riscv::register::time::read64();
    println!("<< Test-kernel: Current time: {:x}", time_start);
    let time_end = riscv::register::time::read64();
    if time_end > time_start {
        println!("<< Test-kernel: Time after operation: {:x}", time_end);
    } else {
        println!("!! Test-kernel: SBI test FAILED due to incorrect time counter");
        sbi::shutdown()
    }
}

pub extern "C" fn rust_trap_exception() {
    let cause = scause::read().cause();
    println!("<< Test-kernel: Value of scause: {:?}", cause);
    if cause != Trap::Exception(Exception::IllegalInstruction) {
        println!("!! Test-kernel: Wrong cause associated to illegal instruction");
        sbi::shutdown()
    }
    println!("<< Test-kernel: Illegal exception delegate success");
    sepc::write(sepc::read().wrapping_add(4));
}

use core::panic::PanicInfo;

#[cfg_attr(not(test), panic_handler)]
#[allow(unused)]
fn panic(info: &PanicInfo) -> ! {
    println!("!! Test-kernel: {}", info);
    println!("!! Test-kernel: SBI test FAILED due to panic");
    sbi::reset(sbi::RESET_TYPE_SHUTDOWN, sbi::RESET_REASON_SYSTEM_FAILURE);
    loop {}
}


const PER_HART_STACK_SIZE: usize = 0x10000;
const BOOT_STACK_SIZE: usize = PER_HART_STACK_SIZE * 2;
static mut BOOT_STACK: [u8; BOOT_STACK_SIZE] = [0; BOOT_STACK_SIZE];

#[naked]
#[link_section = ".text.entry"]
#[export_name = "_start"]
unsafe extern "C" fn entry() -> ! {
    core::arch::asm!(
    // 1. set sp
    // sp = bootstack + (hartid + 1) * HART_STACK_SIZE
    "
    la      sp, {boot_stack}
    li      t0, {per_hart_stack_size}
    addi    t1, a0, 1
1:  add     sp, sp, t0
    addi    t1, t1, -1
    bnez    t1, 1b
    ",
    // 2. jump to rust_main (absolute address)
    "j      {rust_main}", 
    boot_stack = sym BOOT_STACK,
    per_hart_stack_size = const PER_HART_STACK_SIZE,
    rust_main = sym rust_main,
    options(noreturn))
}

#[cfg(target_pointer_width = "128")]
macro_rules! define_store_load {
    () => {
        ".altmacro
        .macro STORE reg, offset
            sq  \\reg, \\offset* {REGBYTES} (sp)
        .endm
        .macro LOAD reg, offset
            lq  \\reg, \\offset* {REGBYTES} (sp)
        .endm"
    };
}

#[cfg(target_pointer_width = "64")]
macro_rules! define_store_load {
    () => {
        ".altmacro
        .macro STORE reg, offset
            sd  \\reg, \\offset* {REGBYTES} (sp)
        .endm
        .macro LOAD reg, offset
            ld  \\reg, \\offset* {REGBYTES} (sp)
        .endm"
    };
}

#[cfg(target_pointer_width = "32")]
macro_rules! define_store_load {
    () => {
        ".altmacro
        .macro STORE reg, offset
            sw  \\reg, \\offset* {REGBYTES} (sp)
        .endm
        .macro LOAD reg, offset
            lw  \\reg, \\offset* {REGBYTES} (sp)
        .endm"
    };
}

#[naked]
#[link_section = ".text"]
unsafe extern "C" fn start_trap() {
    core::arch::asm!(define_store_load!(), "
    .p2align 2
    addi    sp, sp, -16 * {REGBYTES}
    STORE   ra, 0
    STORE   t0, 1
    STORE   t1, 2
    STORE   t2, 3
    STORE   t3, 4
    STORE   t4, 5
    STORE   t5, 6
    STORE   t6, 7
    STORE   a0, 8
    STORE   a1, 9
    STORE   a2, 10
    STORE   a3, 11
    STORE   a4, 12
    STORE   a5, 13
    STORE   a6, 14
    STORE   a7, 15
    mv      a0, sp
    call    {rust_trap_exception}
    LOAD    ra, 0
    LOAD    t0, 1
    LOAD    t1, 2
    LOAD    t2, 3
    LOAD    t3, 4
    LOAD    t4, 5
    LOAD    t5, 6
    LOAD    t6, 7
    LOAD    a0, 8
    LOAD    a1, 9
    LOAD    a2, 10
    LOAD    a3, 11
    LOAD    a4, 12
    LOAD    a5, 13
    LOAD    a6, 14
    LOAD    a7, 15
    addi    sp, sp, 16 * {REGBYTES}
    sret
    ",
    REGBYTES = const core::mem::size_of::<usize>(),
    rust_trap_exception = sym rust_trap_exception,
    options(noreturn))
}

#![no_std]
#![no_main]
#![feature(naked_functions, asm_const, asm_sym)]
#![feature(generator_trait)]
#![feature(default_alloc_error_handler)]
#![feature(ptr_metadata)]
#![allow(dead_code)]

extern crate alloc;

mod device_tree;
mod early_trap;
mod execute;
mod feature;
mod hart_csr_utils;
mod peripheral;
mod runtime;

use core::panic::PanicInfo;
use rustsbi::println;
use buddy_system_allocator::LockedHeap;

#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap<32> = LockedHeap::<32>::empty();

const SBI_HEAP_SIZE: usize = 64 * 1024; // 64KiB
#[link_section = ".bss.uninit"]
static mut HEAP_SPACE: [u8; SBI_HEAP_SIZE] = [0; SBI_HEAP_SIZE];

const PER_HART_STACK_SIZE: usize = 4 * 4096; // 16KiB
const SBI_STACK_SIZE: usize = 2 * PER_HART_STACK_SIZE; // 2 harts
#[link_section = ".bss.uninit"]
static mut SBI_STACK: [u8; SBI_STACK_SIZE] = [0; SBI_STACK_SIZE];

#[panic_handler]
fn on_panic(info: &PanicInfo) -> ! {
    let hart_id = riscv::register::mhartid::read();
    println!("[rustsbi-panic] hart {} {}", hart_id, info); // [rustsbi-panic] hart 0 panicked at xxx
    loop {}
}

static DEVICE_TREE: &'static [u8] = include_bytes!("jh7100-starfive-visionfive-v1.dtb");
static KERNEL: &'static [u8] = include_bytes!("test-kernel.bin");

extern "C" fn rust_main(hart_id: usize) {
    let opaque = DEVICE_TREE.as_ptr() as usize;
    let uart = unsafe { peripheral::Uart::preloaded_uart0() };
    let clint = peripheral::Clint::new(0x2000000 as *mut u8);
    
    early_trap::init(hart_id);
    
    if hart_id == 0 {
        init_bss();
        init_heap(); // 必须先加载堆内存，才能使用rustsbi框架
        init_rustsbi_stdio(uart);
        unsafe {
            core::ptr::copy(KERNEL.as_ptr(), 0x8020_0000 as *mut u8, KERNEL.len());
        }
        println!("[rustsbi] RustSBI version {}", rustsbi::VERSION);
        println!("{}", rustsbi::LOGO);
        println!(
            "[rustsbi] Implementation: RustSBI-JH7100 Version {}",
            env!("CARGO_PKG_VERSION")
        );
        init_rustsbi_clint(clint);
        if let Err(e) = unsafe { device_tree::parse_device_tree(opaque) } {
            println!("[rustsbi] warning: choose from device tree error, {}", e);
        }
        println!(
            "[rustsbi] enter supervisor 0x80200000, opaque register {:#x}",
            opaque
        );
        clint.send_soft(1);
    } else {
        pause(clint);
        // 不是初始化核，先暂停
    }

    // TODO: jump to a confuse address and print pmp panic if set pmp
    set_pmp();
    delegate_interrupt_exception();
    if hart_id == 0 {
        hart_csr_utils::print_hart_csrs();
    }

    runtime::init();
    
    // TODO: Instruction Fault when run at 0x8020_0000
    execute::execute_supervisor(0x8020_0000, hart_id, opaque);
}

fn set_pmp() {
    // todo: 根据QEMU的loader device等等，设置这里的权限配置
    // read fdt tree value, parse, and calculate proper pmp configuration for this device tree (issue #7)
    // integrate with `count_harts`
    //
    // Qemu MMIO config ref: https://github.com/qemu/qemu/blob/master/hw/riscv/virt.c#L46
    //
    // About PMP:
    //
    // CSR: pmpcfg0(0x3A0)~pmpcfg15(0x3AF); pmpaddr0(0x3B0)~pmpaddr63(0x3EF)
    // pmpcfg packs pmp entries each of which is of 8-bit
    // on RV64 only even pmpcfg CSRs(0,2,...,14) are available, each of which contains 8 PMP
    // entries
    // every pmp entry and its corresponding pmpaddr describe a pmp region
    //
    // layout of PMP entries:
    // ------------------------------------------------------
    //  7   |   [5:6]   |   [3:4]   |   2   |   1   |   0   |
    //  L   |   0(WARL) |   A       |   X   |   W   |   R   |
    // ------------------------------------------------------
    // A = OFF(0), disabled;
    // A = TOR(top of range, 1), match address y so that pmpaddr_{i-1}<=y<pmpaddr_i irrespective of
    // the value pmp entry i-1
    // A = NA4(naturally aligned 4-byte region, 2), only support a 4-byte pmp region
    // A = NAPOT(naturally aligned power-of-two region, 3), support a >=8-byte pmp region
    // When using NAPOT to match a address range [S,S+L), then the pmpaddr_i should be set to (S>>2)|((L>>2)-1)
    let calc_pmpaddr = |start_addr: usize, length: usize| (start_addr >> 2) | ((length >> 2) - 1);
    let mut pmpcfg0: usize = 0;
    // pmp region 0: RW, A=NAPOT, address range {0x1000_0000, 0x800_0000}, peripherals CSR
    pmpcfg0 |= 0b11011;
    let pmpaddr0 = calc_pmpaddr(0x1000_0000, 0x800_0000);
    // pmp region 1: RW, A=NAPOT, address range {0x200_0000, 0x1_0000}, CLINT
    pmpcfg0 |= 0b11011 << 8;
    let pmpaddr1 = calc_pmpaddr(0x200_0000, 0x1_0000);
    // pmp region 2: RWX, A=NAPOT, address range {0x8000_0000, 0x2_0000_0000}, DRAM
    pmpcfg0 |= 0b11111 << 16;
    let pmpaddr2 = calc_pmpaddr(0x8000_0000, 0x2_0000_0000);
    unsafe {
        core::arch::asm!("csrw  pmpcfg0, {}",
             "csrw  pmpaddr0, {}",
             "csrw  pmpaddr1, {}",
             "csrw  pmpaddr2, {}",
             "sfence.vma",
             in(reg) pmpcfg0,
             in(reg) pmpaddr0,
             in(reg) pmpaddr1,
             in(reg) pmpaddr2,
        );
    }
}


fn init_bss() {
    extern "C" {
        static mut ebss: u32;
        static mut sbss: u32;
        static mut edata: u32;
        static mut sdata: u32;
        static sidata: u32;
    }
    unsafe {
        r0::zero_bss(&mut sbss, &mut ebss);
        r0::init_data(&mut sdata, &mut edata, &sidata);
    }
}

fn init_rustsbi_stdio(uart: peripheral::Uart) {
    use rustsbi::legacy_stdio::init_legacy_stdio_embedded_hal;
    init_legacy_stdio_embedded_hal(uart);
}

fn init_rustsbi_clint(clint: peripheral::Clint) {
    rustsbi::init_ipi(clint);
    rustsbi::init_timer(clint);
}

fn delegate_interrupt_exception() {
    use riscv::register::{medeleg, mideleg, mie};
    unsafe {
        mideleg::set_sext();
        mideleg::set_stimer();
        mideleg::set_ssoft();
        mideleg::set_uext();
        mideleg::set_utimer();
        mideleg::set_usoft();
        medeleg::set_instruction_misaligned();
        medeleg::set_breakpoint();
        medeleg::set_user_env_call();
        medeleg::set_instruction_page_fault();
        medeleg::set_load_page_fault();
        medeleg::set_store_page_fault();
        medeleg::set_instruction_fault();
        medeleg::set_load_fault();
        medeleg::set_store_fault();
        mie::set_mext();
        // 不打开mie::set_mtimer
        mie::set_msoft();
    }
}

pub fn pause(clint: peripheral::Clint) {
    use riscv::asm::wfi;
    use riscv::register::{mhartid, mie, mip};
    unsafe {
        let hartid = mhartid::read();
        clint.clear_soft(hartid); // Clear IPI
        mip::clear_msoft(); // clear machine software interrupt flag
        let prev_msoft = mie::read().msoft();
        mie::set_msoft(); // Start listening for software interrupts
        loop {
            wfi();
            if mip::read().msoft() {
                break;
            }
        }
        if !prev_msoft {
            mie::clear_msoft(); // Stop listening for software interrupts
        }
        clint.clear_soft(hartid); // Clear IPI
    }
}

#[inline]
fn init_heap() {
    unsafe {
        HEAP_ALLOCATOR
            .lock()
            .init(HEAP_SPACE.as_ptr() as usize, SBI_HEAP_SIZE);
    }
}

#[naked]
#[link_section = ".text.entry"]
#[export_name = "_start"]
unsafe extern "C" fn entry() -> ! {
    core::arch::asm!(
    // 1. set sp
    // sp = bootstack + (hart_id + 1) * HART_STACK_SIZE
    "
    la      sp, {stack}
    li      t0, {per_hart_stack_size}
    csrr    t1, mhartid
    addi    t2, t1, 1
1:  add     sp, sp, t0
    addi    t2, t2, -1
    bnez    t2, 1b
    ",
    // 2. jump to main function (absolute address)
    "j   {rust_main}",
    per_hart_stack_size = const PER_HART_STACK_SIZE,
    stack = sym SBI_STACK,
    rust_main = sym rust_main,
    options(noreturn))
}

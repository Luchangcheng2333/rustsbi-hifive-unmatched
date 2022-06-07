#![no_std]
#![no_main]
#![feature(naked_functions, asm_const, asm_sym)]
#![feature(generator_trait)]
#![feature(default_alloc_error_handler)]
#![feature(ptr_metadata)]

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

#[panic_handler]
fn on_panic(info: &PanicInfo) -> ! {
    let hart_id = riscv::register::mhartid::read();
    println!("[rustsbi-panic] hart {} {}", hart_id, info); // [rustsbi-panic] hart 0 panicked at xxx
    loop {}
}

static DEVICE_TREE: &'static [u8] = include_bytes!("jh7100-starfive-visionfive-v1.dtb");

extern "C" fn rust_main(hart_id: usize) {
    let opaque = DEVICE_TREE.as_ptr() as usize;
    let uart = unsafe { peripheral::Uart::preloaded_uart0() };
    let clint = peripheral::Clint::new(0x2000000 as *mut u32);
    
    early_trap::init(hart_id);
    if hart_id == 0 {
        init_bss();
        init_heap(); // 必须先加载堆内存，才能使用rustsbi框架
        init_rustsbi_stdio(uart);
        
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
            "[rustsbi] enter supervisor 0x8002_0000, opaque register {:#x}",
            opaque
        );
        hart_csr_utils::print_hart0_csrs();
        for target_hart_id in 0..2 {
            if target_hart_id != 0 {
                // clint.send_soft(target_hart_id);
            }
        }
    } else {
        pause(clint);
        // 不是初始化核，先暂停
        if hart_id == 1 {
            hart_csr_utils::print_hartn_csrs();
        }
        
    }
    delegate_interrupt_exception();
    runtime::init();
    execute::execute_supervisor(0x8002_0000, riscv::register::mhartid::read(), opaque);
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

const SBI_HEAP_SIZE: usize = 6 * 1024; // 8KiB
#[link_section = ".bss.uninit"]
static mut HEAP_SPACE: [u8; SBI_HEAP_SIZE] = [0; SBI_HEAP_SIZE];

use buddy_system_allocator::LockedHeap;

#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap<32> = LockedHeap::<32>::empty();

#[inline]
fn init_heap() {
    unsafe {
        HEAP_ALLOCATOR
            .lock()
            .init(HEAP_SPACE.as_ptr() as usize, SBI_HEAP_SIZE);
    }
}

const PER_HART_STACK_SIZE: usize = 3 * 4096; // 8KiB
const SBI_STACK_SIZE: usize = 2 * PER_HART_STACK_SIZE; // 2 harts
#[link_section = ".bss.uninit"]
static mut SBI_STACK: [u8; SBI_STACK_SIZE] = [0; SBI_STACK_SIZE];

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

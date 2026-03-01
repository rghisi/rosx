pub fn raw_syscall(num: usize, arg1: usize, arg2: usize, arg3: usize) -> usize {
    let result: usize;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("eax") num,
            in("ebx") arg1,
            in("ecx") arg2,
            in("edx") arg3,
            lateout("eax") result,
            options(nostack, preserves_flags)
        );
    }
    result
}

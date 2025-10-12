use runnable::Runnable;
use state::{CURRENT_TASK, MAIN_THREAD_PTR};

pub(crate) extern "C" fn task_wrapper(actual_entry: usize) {
    let task_fn: fn() = unsafe { core::mem::transmute(actual_entry) };
    task_fn();

    unsafe {
        if let Some(mut task) = CURRENT_TASK.take() {
            task.set_terminated();
            CURRENT_TASK = Some(task);
        }
    }

    crate::kernel::task_yield();
}

pub(crate) extern "C" fn main_thread_wrapper() -> ! {
    unsafe {
        if let Some(ptr) = MAIN_THREAD_PTR {
            let main_thread = &mut *ptr;
            main_thread.run();
        }
    }

    loop {}
}
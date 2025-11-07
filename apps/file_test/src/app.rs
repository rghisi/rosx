
use usrlib::println;
use usrlib::syscall::Syscall;
use system::message::{Message, MessageType};
use alloc::vec;

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct FileHandle {
    pub index: u8,
    pub generation: u8,
}

pub fn main() {
    println!("File Test App Started");

    let file_path = "/test.txt";
    let write_buf = "Hello from file_test app!";
    let mut read_buf = [0u8; 30];

    // Open file
    let open_message = Message {
        message_type: MessageType::FileOpen,
        data: file_path.as_bytes().to_vec(),
    };
    let open_result = Syscall::syscall(&open_message);
    if open_result == usize::MAX {
        println!("Failed to open file");
        return;
    }
    let handle = open_result as u16;
    println!("File opened successfully");

    // Write to file
    let mut write_data = vec![];
    write_data.extend_from_slice(&handle.to_ne_bytes());
    write_data.extend_from_slice(&(write_buf.as_ptr() as usize).to_ne_bytes());
    write_data.extend_from_slice(&write_buf.len().to_ne_bytes());
    let write_message = Message {
        message_type: MessageType::FileWrite,
        data: write_data,
    };
    let write_result = Syscall::syscall(&write_message);
    if write_result == usize::MAX {
        println!("Failed to write to file");
        return;
    }
    println!("Wrote {} bytes to file", write_result);

    // Read from file
    let mut read_data = vec![];
    read_data.extend_from_slice(&handle.to_ne_bytes());
    read_data.extend_from_slice(&(read_buf.as_mut_ptr() as usize).to_ne_bytes());
    read_data.extend_from_slice(&read_buf.len().to_ne_bytes());
    let read_message = Message {
        message_type: MessageType::FileRead,
        data: read_data,
    };
    let read_result = Syscall::syscall(&read_message);
    if read_result == usize::MAX {
        println!("Failed to read from file");
        return;
    }
    println!("Read {} bytes from file", read_result);

    // Close file
    let mut close_data = vec![];
    close_data.extend_from_slice(&handle.to_ne_bytes());
    let close_message = Message {
        message_type: MessageType::FileClose,
        data: close_data,
    };
    let close_result = Syscall::syscall(&close_message);
    if close_result == usize::MAX {
        println!("Failed to close file");
        return;
    }
    println!("File closed successfully");

    let content = core::str::from_utf8(&read_buf[..read_result]).unwrap();
    println!("File content: '{}'", content);

    if content == write_buf {
        println!("File content matches what was written!");
    } else {
        println!("File content does NOT match what was written!");
    }
}

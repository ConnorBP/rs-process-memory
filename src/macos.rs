use libc;
use mach;

use self::mach::kern_return::{kern_return_t, KERN_SUCCESS};
use self::mach::message::mach_msg_type_number_t;
use self::mach::port::{mach_port_name_t, mach_port_t, MACH_PORT_NULL};
use self::mach::vm_types::{mach_vm_address_t, mach_vm_offset_t, mach_vm_size_t};
use libc::{c_int, pid_t};
use std::process::Child;

use super::{CopyAddress, PutAddress, TryIntoProcessHandle};

#[allow(non_camel_case_types)]
type vm_map_t = mach_port_t;
#[allow(non_camel_case_types)]
type vm_address_t = mach_vm_address_t;
#[allow(non_camel_case_types)]
type vm_size_t = mach_vm_size_t;

/// On OS X a `Pid` is just a `libc::pid_t`.
pub type Pid = pid_t;
/// On OS X a `ProcessHandle` is a mach port.
pub type ProcessHandle = mach_port_name_t;

extern "C" {
    /// Parameters
    ///  - target_task: The task that we will read from
    ///  - address: The address on the foreign task that we will read
    ///  - size: The number of bytes we want to read
    ///  - data: The local address to read into
    ///  - outsize: The actual size we read
    fn vm_read_overwrite(
        target_task: vm_map_t,
        address: vm_address_t,
        size: vm_size_t,
        data: vm_address_t,
        outsize: &mut vm_size_t,
    ) -> kern_return_t;
    /// Parameters:
    ///  - target_task: The task to which we will write
    ///  - address: The address on the foreign task that we will write to
    ///  - data: The local address of the data we're putting in
    ///  - data_count: The number of bytes we are copying
    fn mach_vm_write(
        target_task: vm_map_t,
        address: vm_address_t,
        data: mach_vm_offset_t,
        data_count: mach_msg_type_number_t,
    ) -> kern_return_t;
}

/// A small wrapper around `task_for_pid`, which taskes a pid returns the mach port representing its task.
fn task_for_pid(pid: Pid) -> std::io::Result<mach_port_name_t> {
    let mut task: mach_port_name_t = MACH_PORT_NULL;

    unsafe {
        let result =
            mach::traps::task_for_pid(mach::traps::mach_task_self(), pid as c_int, &mut task);
        if result != KERN_SUCCESS {
            return Err(std::io::Error::last_os_error());
        }
    }

    Ok(task)
}

/// `Pid` can be turned into a `ProcessHandle` with `task_for_pid`.
impl TryIntoProcessHandle for Pid {
    fn try_into_process_handle(&self) -> std::io::Result<ProcessHandle> {
        task_for_pid(*self)
    }
}

/// This `TryIntoProcessHandle` impl simply calls the `TryIntoProcessHandle` impl for `Pid`.
///
/// Unfortunately spawning a process on OS X does not hand back a mach
/// port by default (you have to jump through several hoops to get at it),
/// so there's no simple implementation of `TryIntoProcessHandle` for
/// `std::process::Child`. This implementation is just provided for symmetry
/// with other platforms to make writing cross-platform code easier.
///
/// Ideally we would provide an implementation of `std::process::Command::spawn`
/// that jumped through those hoops and provided the task port.
impl TryIntoProcessHandle for Child {
    fn try_into_process_handle(&self) -> std::io::Result<ProcessHandle> {
        Pid::try_into_process_handle(&(self.id() as _))
    }
}

/// Here we use `mach_vm_write` to write a buffer to some arbitrary address on a process.
impl PutAddress for ProcessHandle {
    fn put_address(&self, addr: usize, buf: &[u8]) -> std::io::Result<()> {
        let result = unsafe { mach_vm_write(*self, addr as _, buf.as_ptr() as _, buf.len() as _) };
        if result != KERN_SUCCESS {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

/// Use `vm_read_overwrite` to read memory from another process on OS X.
///
/// We use `vm_read_overwrite` instead of `vm_read` because it can handle non-aligned reads and
/// won't read an entire page.
impl CopyAddress for ProcessHandle {
    fn copy_address(&self, addr: usize, buf: &mut [u8]) -> std::io::Result<()> {
        let mut read_len: u64 = 0;
        let result = unsafe {
            vm_read_overwrite(
                *self,
                addr as _,
                buf.len() as _,
                buf.as_ptr() as _,
                &mut read_len,
            )
        };

        if result != KERN_SUCCESS {
            return Err(std::io::Error::last_os_error());
        }

        if read_len == buf.len() as _ {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                format!(
                    "Mismatched read sizes for `vm_read_overwrite` (expected {}, got {})",
                    buf.len(),
                    read_len
                ),
            ))
        }
    }
}
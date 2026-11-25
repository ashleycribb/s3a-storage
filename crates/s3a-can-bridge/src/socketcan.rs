//! Live SocketCAN reader. Compiled only with the `socketcan` feature.
//! There is no mock fallback in this module.

use crate::error::CanError;
use crate::frame::CanFrame;

/// Live reader bound to a named SocketCAN interface (for example `can0`
/// or `vcan0`). Opening a missing interface fails closed.
#[derive(Debug)]
pub struct SocketCanReader {
    fd: libc::c_int,
    interface: String,
}

impl SocketCanReader {
    /// Bind a raw CAN socket to `interface`. On any failure, including a
    /// missing interface, returns `SocketCanOpenFailed`. Never substitutes
    /// a mock source.
    pub fn open(interface: &str) -> Result<Self, CanError> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = interface;
            return Err(CanError::SocketCanOpenFailed);
        }
        #[cfg(target_os = "linux")]
        {
            open_linux(interface)
        }
    }

    /// Read the next frame from the interface.
    pub fn next_frame(&mut self) -> Result<CanFrame, CanError> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = self;
            Err(CanError::SocketCanOpenFailed)
        }
        #[cfg(target_os = "linux")]
        {
            read_linux(self)
        }
    }
}

impl Drop for SocketCanReader {
    fn drop(&mut self) {
        if self.fd >= 0 {
            unsafe {
                libc::close(self.fd);
            }
            self.fd = -1;
        }
    }
}

#[cfg(target_os = "linux")]
fn open_linux(interface: &str) -> Result<SocketCanReader, CanError> {
    if interface.is_empty() || interface.contains('\0') || interface.len() >= libc::IFNAMSIZ {
        return Err(CanError::SocketCanOpenFailed);
    }
    let fd = unsafe { libc::socket(libc::AF_CAN, libc::SOCK_RAW, libc::CAN_RAW as libc::c_int) };
    if fd < 0 {
        return Err(CanError::SocketCanOpenFailed);
    }
    let mut ifr: libc::ifreq = unsafe { std::mem::zeroed() };
    for (i, b) in interface.as_bytes().iter().enumerate() {
        ifr.ifr_name[i] = *b as libc::c_char;
    }
    let ret = unsafe { libc::ioctl(fd, libc::SIOCGIFINDEX as _, &mut ifr as *mut _) };
    if ret < 0 {
        unsafe { libc::close(fd) };
        return Err(CanError::SocketCanOpenFailed);
    }
    let ifindex = unsafe { ifr.ifr_ifru.ifru_ifindex };
    let addr = libc::sockaddr_can {
        can_family: libc::AF_CAN as libc::sa_family_t,
        can_ifindex: ifindex,
        can_addr: unsafe { std::mem::zeroed() },
    };
    let bind_ret = unsafe {
        libc::bind(
            fd,
            &addr as *const libc::sockaddr_can as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_can>() as libc::socklen_t,
        )
    };
    if bind_ret < 0 {
        unsafe { libc::close(fd) };
        return Err(CanError::SocketCanOpenFailed);
    }
    Ok(SocketCanReader {
        fd,
        interface: interface.to_string(),
    })
}

#[cfg(target_os = "linux")]
fn read_linux(reader: &SocketCanReader) -> Result<CanFrame, CanError> {
    let mut frame: libc::can_frame = unsafe { std::mem::zeroed() };
    let n = unsafe {
        libc::read(
            reader.fd,
            &mut frame as *mut libc::can_frame as *mut libc::c_void,
            std::mem::size_of::<libc::can_frame>(),
        )
    };
    if n != std::mem::size_of::<libc::can_frame>() as isize {
        return Err(CanError::SocketCanOpenFailed);
    }
    let ext = (frame.can_id & libc::CAN_EFF_FLAG) != 0;
    let id = if ext {
        frame.can_id & libc::CAN_EFF_MASK
    } else {
        frame.can_id & libc::CAN_SFF_MASK
    };
    let dlc = frame.can_dlc;
    if dlc > 8 {
        return Err(CanError::SocketCanOpenFailed);
    }
    let mut data = [0u8; 8];
    data[..dlc as usize].copy_from_slice(&frame.data[..dlc as usize]);
    Ok(CanFrame {
        timestamp_ns: 0,
        interface: reader.interface.clone(),
        id,
        extended: ext,
        dlc,
        data,
    })
}

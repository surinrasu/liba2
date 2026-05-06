use std::net::UdpSocket;
use std::os::unix::io::AsRawFd;

pub(super) fn set_socket_qos(socket: &UdpSocket) {
    let fd = socket.as_raw_fd();

    let tos: libc::c_int = 0xB8;
    unsafe {
        let ret = libc::setsockopt(
            fd,
            libc::IPPROTO_IP,
            libc::IP_TOS,
            &tos as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        if ret != 0 {
            tracing::debug!(
                "Failed to set IP_TOS (DSCP EF): errno={}",
                std::io::Error::last_os_error()
            );
        }
    }

    unsafe {
        let buf_size: libc::c_int = 1024 * 1024;
        let ret = libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            &buf_size as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        if ret != 0 {
            tracing::debug!(
                "Failed to set SO_SNDBUF: errno={}",
                std::io::Error::last_os_error()
            );
        }
    }

    #[cfg(target_os = "linux")]
    unsafe {
        let prio: libc::c_int = 6;
        let ret = libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PRIORITY,
            &prio as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        if ret != 0 {
            tracing::debug!(
                "Failed to set SO_PRIORITY: errno={}",
                std::io::Error::last_os_error()
            );
        }
    }
}

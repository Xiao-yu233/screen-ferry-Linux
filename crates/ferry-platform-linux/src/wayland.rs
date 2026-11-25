//! Wayland 连接、自定义协议和事件分发。

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Duration;

use wayland_client::Connection;

use ferry_platform::Error;

pub(crate) mod virtual_keyboard {
    #[allow(clippy::single_component_path_imports)]
    use wayland_client;
    use wayland_client::protocol::{wl_seat, wl_seat::WlSeat};
    use wayland_client::{Dispatch, QueueHandle};

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("src/protocols/virtual-keyboard-unstable-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("src/protocols/virtual-keyboard-unstable-v1.xml");

    pub use self::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
    pub use self::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;

    pub fn create(
        manager: &ZwpVirtualKeyboardManagerV1,
        seat: &WlSeat,
        qh: &QueueHandle<impl Dispatch<ZwpVirtualKeyboardV1, ()> + 'static>,
    ) -> ZwpVirtualKeyboardV1 {
        manager.create_virtual_keyboard(seat, qh, ())
    }
}

pub(crate) mod virtual_pointer {
    #[allow(clippy::single_component_path_imports)]
    use wayland_client;
    use wayland_client::protocol::{wl_seat, wl_seat::WlSeat};
    use wayland_client::{Dispatch, QueueHandle};

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("src/protocols/virtual-pointer-unstable-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("src/protocols/virtual-pointer-unstable-v1.xml");

    pub use self::zwp_virtual_pointer_manager_v1::ZwpVirtualPointerManagerV1;
    pub use self::zwp_virtual_pointer_v1::ZwpVirtualPointerV1;

    pub fn create(
        manager: &ZwpVirtualPointerManagerV1,
        seat: &WlSeat,
        qh: &QueueHandle<impl Dispatch<ZwpVirtualPointerV1, ()> + 'static>,
    ) -> ZwpVirtualPointerV1 {
        manager.create_virtual_pointer(seat, qh, ())
    }
}

pub(crate) fn error(call: &'static str, detail: impl std::fmt::Display) -> Error {
    Error::Os {
        call,
        detail: detail.to_string(),
    }
}

pub(crate) fn connect() -> Result<Connection, Error> {
    Connection::connect_to_env().map_err(|e| error("connect_to_env", e))
}

pub(crate) struct Wake {
    read: OwnedFd,
    write: OwnedFd,
}

impl Wake {
    pub(crate) fn new() -> io::Result<Self> {
        let mut fds = [0; 2];
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            read: unsafe { OwnedFd::from_raw_fd(fds[0]) },
            write: unsafe { OwnedFd::from_raw_fd(fds[1]) },
        })
    }

    pub(crate) fn notify(&self) {
        let _ = unsafe { libc::write(self.write.as_raw_fd(), b"x".as_ptr().cast(), 1) };
    }

    pub(crate) fn clear(&self) {
        let mut byte = [0u8; 64];
        while unsafe { libc::read(self.read.as_raw_fd(), byte.as_mut_ptr().cast(), byte.len()) } > 0
        {
        }
    }
}

pub(crate) fn wait(
    conn: &Connection,
    wake: &Wake,
    stop: &std::sync::atomic::AtomicBool,
) -> io::Result<bool> {
    if stop.load(std::sync::atomic::Ordering::SeqCst) {
        return Ok(false);
    }
    conn.flush().map_err(|error| match error {
        wayland_client::backend::WaylandError::Io(error) => error,
        error => io::Error::other(error),
    })?;
    let Some(guard) = conn.prepare_read() else {
        return Ok(true);
    };
    let mut fds = [
        libc::pollfd {
            fd: conn.backend().poll_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: wake.read.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    loop {
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, 100) };
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        break;
    }
    let connection_events = fds[0].revents;
    if connection_events & libc::POLLNVAL != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Wayland 连接文件描述符无效",
        ));
    }
    if connection_events & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
        guard.read().map_err(|error| match error {
            wayland_client::backend::WaylandError::Io(error) => error,
            error => io::Error::other(error),
        })?;
    } else if connection_events != 0 {
        return Err(io::Error::other("Wayland 连接异常"));
    }
    if fds[1].revents & libc::POLLIN != 0 {
        wake.clear();
    }
    Ok(true)
}

pub(crate) fn timestamp() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u32
}

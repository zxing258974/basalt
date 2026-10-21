use crate::net::Socket;

use std::io::{self, Read, Write};
use std::task::{Context, Poll};

use futures_util::future;

pub struct StdSocket<S> {
    pub socket: S,
    wants_read: bool,
    wants_write: bool,
}

impl<S: Socket> StdSocket<S> {
    pub fn new(socket: S) -> Self {
        Self {
            socket,
            wants_read: false,
            wants_write: false,
        }
    }

    /// Returns `Ready` if a previously blocked read _or_ write may now proceed.
    ///
    /// If both a read and a write were attempted, to avoid deadlocks this returns `Ready`
    /// when _either_ direction is ready, not necessarily both.
    //
    // basalt 补丁:照搬 sqlx 0.9.0 的实现。原来先等可写、再等可读:rustls 的 `complete_io`
    // 写被阻塞后还会试着读一次,两个标志都留着;这时可写先到,就转去只等可读,而服务器在等
    // 这个请求剩下的部分、什么也不发,两边互等卡死(大批量 INSERT 时会碰上)。
    pub fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // Return `Ready` without waiting if the caller hasn't tried to do I/O in either direction.
        let mut ready = !(self.wants_read || self.wants_write);

        if self.wants_write && self.socket.poll_write_ready(cx)?.is_ready() {
            self.wants_write = false;
            ready = true;
        }

        if self.wants_read && self.socket.poll_read_ready(cx)?.is_ready() {
            self.wants_read = false;
            ready = true;
        }

        if ready {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    pub async fn ready(&mut self) -> io::Result<()> {
        future::poll_fn(|cx| self.poll_ready(cx)).await
    }
}

impl<S: Socket> Read for StdSocket<S> {
    fn read(&mut self, mut buf: &mut [u8]) -> io::Result<usize> {
        self.wants_read = true;
        let read = self.socket.try_read(&mut buf)?;
        self.wants_read = false;

        Ok(read)
    }
}

impl<S: Socket> Write for StdSocket<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.wants_write = true;
        let written = self.socket.try_write(buf)?;
        self.wants_write = false;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        // NOTE: TCP sockets and unix sockets are both no-ops for flushes
        Ok(())
    }
}

// basalt 补丁:读写都被阻塞时的就绪判断(见 vendor/README.md)。
#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::ReadBuf;
    use std::task::Waker;

    /// 读写都会 WouldBlock 的套接字;两个方向的就绪由测试指定。
    struct Blocked {
        writable: bool,
        readable: bool,
    }

    impl Socket for Blocked {
        fn try_read(&mut self, _: &mut dyn ReadBuf) -> io::Result<usize> {
            Err(io::ErrorKind::WouldBlock.into())
        }

        fn try_write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::WouldBlock.into())
        }

        fn poll_read_ready(&mut self, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            if self.readable {
                Poll::Ready(Ok(()))
            } else {
                Poll::Pending
            }
        }

        fn poll_write_ready(&mut self, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            if self.writable {
                Poll::Ready(Ok(()))
            } else {
                Poll::Pending
            }
        }

        fn poll_shutdown(&mut self, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    /// rustls 的 `complete_io` 写被阻塞后还会读一次,两个方向都留下「在等」。
    fn both_blocked(writable: bool, readable: bool) -> StdSocket<Blocked> {
        let mut socket = StdSocket::new(Blocked { writable, readable });
        assert!(socket.write(b"x").is_err());
        assert!(socket.read(&mut [0; 1]).is_err());
        socket
    }

    #[test]
    fn writable_is_enough_when_both_directions_blocked() {
        // 服务器在等这个请求剩下的部分,不会发任何东西:只要可写就得回去接着写,不能再等可读。
        let mut socket = both_blocked(true, false);
        let mut cx = Context::from_waker(Waker::noop());
        assert!(socket.poll_ready(&mut cx).is_ready());
    }

    #[test]
    fn readable_is_enough_when_both_directions_blocked() {
        let mut socket = both_blocked(false, true);
        let mut cx = Context::from_waker(Waker::noop());
        assert!(socket.poll_ready(&mut cx).is_ready());
    }

    #[test]
    fn ready_direction_stops_being_waited_on() {
        // 就绪的方向要清掉标志,否则下一次还会因为它返回 Ready,调用方原地空转。
        let mut socket = both_blocked(false, true);
        let mut cx = Context::from_waker(Waker::noop());
        assert!(socket.poll_ready(&mut cx).is_ready());
        assert!(socket.poll_ready(&mut cx).is_pending());
    }

    #[test]
    fn waits_while_neither_direction_is_ready() {
        let mut socket = both_blocked(false, false);
        let mut cx = Context::from_waker(Waker::noop());
        assert!(socket.poll_ready(&mut cx).is_pending());
    }
}

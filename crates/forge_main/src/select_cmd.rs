use std::io;

#[cfg(unix)]
pub(crate) fn redirect_stdin_to_tty() -> io::Result<()> {
    use std::os::unix::io::AsRawFd;

    let tty = std::fs::File::open("/dev/tty")?;
    let tty_fd = tty.as_raw_fd();

    unsafe {
        if libc::dup2(tty_fd, 0) == -1 {
            return Err(io::Error::last_os_error());
        }
    }

    Ok(())
}

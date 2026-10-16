use crate::terminal::Theme;

fn background(response: &str) -> Option<Theme> {
    let remaining = response.split("11;rgb:").nth(1)?;
    let end = remaining.find('\x07').or_else(|| remaining.find("\x1b\\"))?;
    let value = &remaining[..end];
    let channels: Vec<_> = value
        .split('/')
        .map(|channel| {
            let intensity = u32::from_str_radix(channel, 16).ok()?;
            let maximum = 16_f64.powi(channel.len() as i32) - 1.0;
            Some(f64::from(intensity) / maximum)
        })
        .collect::<Option<_>>()?;
    if channels.len() != 3 {
        return None;
    }
    let luminance = channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
    Some(if luminance > 0.5 { Theme::Light } else { Theme::Dark })
}

#[cfg(unix)]
pub(super) fn detect() -> (Option<Theme>, Vec<u8>) {
    use std::{
        io::{self, Write},
        time::{Duration, Instant},
    };
    let mut output = io::stdout();
    if output.write_all(b"\x1b]11;?\x07").and_then(|()| output.flush()).is_err() {
        return (None, Vec::new());
    }
    let deadline = Instant::now() + Duration::from_millis(120);
    let mut response = Vec::new();
    while Instant::now() < deadline {
        let mut descriptor = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout = deadline.saturating_duration_since(Instant::now()).as_millis() as i32;
        // Poll only the terminal descriptor before reading; the query must not block on an unsupported terminal.
        if unsafe { libc::poll(&mut descriptor, 1, timeout) } <= 0 {
            break;
        }
        let mut bytes = [0_u8; 256];
        // The buffer pointer is valid for its full length and read never retains it.
        let count = unsafe { libc::read(libc::STDIN_FILENO, bytes.as_mut_ptr().cast(), bytes.len()) };
        if count <= 0 {
            break;
        }
        response.extend_from_slice(&bytes[..count as usize]);
        if let Some(theme) = background(&String::from_utf8_lossy(&response)) {
            let end = response
                .iter()
                .position(|byte| *byte == 7)
                .map(|index| index + 1)
                .or_else(|| response.windows(2).position(|pair| pair == b"\x1b\\").map(|index| index + 2))
                .unwrap_or(response.len());
            return (Some(theme), response[end..].to_vec());
        }
        if response.first().is_some_and(|byte| *byte != 27) {
            return (None, response);
        }
    }
    (None, Vec::new())
}

#[cfg(not(unix))]
pub(super) fn detect() -> (Option<Theme>, Vec<u8>) {
    (None, Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc_11_backgrounds_and_invalid_responses() {
        assert_eq!(background("\x1b]11;rgb:ffff/eeee/dddd\x1b\\"), Some(Theme::Light));
        assert_eq!(background("\x1b]11;rgb:1e/1e/2e\x07"), Some(Theme::Dark));
        assert_eq!(background("\x1b]11;rgb:invalid/00/00\x07"), None);
        assert_eq!(background("no response"), None);
        assert_eq!(background("\x1b]11;rgb:ffff/ffff/ffff"), None);
    }
}

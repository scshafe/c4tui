use std::process::Command;

fn main() {
    println!(
        "cargo:rustc-env=C4TUI_BUILD_TIME_HHMM={}",
        build_time_hhmm()
    );
}

fn build_time_hhmm() -> String {
    Command::new("date")
        .arg("+%H:%M")
        .output()
        .ok()
        .and_then(|output| {
            output
                .status
                .success()
                .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        })
        .filter(|time| is_hhmm(time))
        .unwrap_or_else(|| "??:??".to_owned())
}

fn is_hhmm(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 5
        && bytes[2] == b':'
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[3].is_ascii_digit()
        && bytes[4].is_ascii_digit()
}

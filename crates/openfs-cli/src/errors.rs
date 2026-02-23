use std::error::Error;

/// Print a user-friendly error message with optional hint.
pub fn print_error(err: &dyn Error) {
    let msg = err.to_string();

    // Try to extract hint from the error message
    let (main_msg, hint) = extract_hint(&msg);

    eprintln!("error: {}", main_msg);

    if let Some(hint_msg) = hint {
        eprintln!("  hint: {}", hint_msg);
    }

    // Walk the error chain for additional context
    let mut source = err.source();
    while let Some(cause) = source {
        eprintln!("  caused by: {}", cause);
        source = cause.source();
    }
}

/// Extract a hint from known error message patterns.
fn extract_hint(msg: &str) -> (&str, Option<&str>) {
    // Check for actionable patterns in the message
    if msg.contains("No configuration file found") {
        return (
            msg,
            Some("Create an openfs.yaml file or use --config to specify one"),
        );
    }
    if msg.contains("No mount found for path") {
        if let Some(hint_start) = msg.find("Check your") {
            let main = msg[..hint_start].trim_end_matches(". ");
            let hint = &msg[hint_start..];
            return (main, Some(hint));
        }
    }
    if msg.contains("read_only: true") {
        if let Some(hint_start) = msg.find("Remove") {
            let main = msg[..hint_start].trim_end_matches(". ");
            let hint = &msg[hint_start..];
            return (main, Some(hint));
        }
    }
    (msg, None)
}

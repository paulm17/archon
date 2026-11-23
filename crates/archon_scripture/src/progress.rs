use indicatif::{ProgressBar, ProgressStyle};

/// Create a standard progress bar with a count style.
pub fn count_bar(total: u64, msg: &str) -> ProgressBar {
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} {msg} [{elapsed_precise}] [{bar:50.cyan/blue}] {pos}/{len} ({eta})",
        )
        .unwrap()
        .progress_chars("=>-"),
    );
    pb.set_message(msg.to_string());
    pb
}

/// Create an indeterminate spinner.
pub fn spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner:.green} {msg} [{elapsed_precise}]")
            .unwrap()
            .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
    pb
}

/// Finish a progress bar with a success message.
pub fn finish_ok(pb: &ProgressBar, msg: &str) {
    pb.finish_with_message(format!("✅ {msg}"));
}

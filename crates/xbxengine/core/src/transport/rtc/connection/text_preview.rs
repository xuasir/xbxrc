pub(crate) fn short_text_preview(payload: &str, max_chars: usize) -> String {
    let mut preview = payload.chars().take(max_chars).collect::<String>();
    if payload.chars().count() > max_chars {
        preview.push_str("...");
    }
    preview
}

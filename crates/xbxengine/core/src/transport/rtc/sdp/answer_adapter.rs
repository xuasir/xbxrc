pub(crate) fn adapt_remote_answer(answer_sdp: &str) -> String {
    // 统一行结束符并剔除空行，避免 Sans-I/O 对 SDP 文本解析受平台换行影响。
    let mut normalized = answer_sdp
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\r\n");
    normalized.push_str("\r\n");
    normalized
}

#[cfg(test)]
mod tests {
    use super::adapt_remote_answer;

    #[test]
    fn adapt_remote_answer_normalizes_line_endings_and_trims_blank_lines() {
        let input = "v=0\n\nm=audio 9 UDP/TLS/RTP/SAVPF 111 \n";
        let output = adapt_remote_answer(input);
        assert_eq!(output, "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n");
    }
}

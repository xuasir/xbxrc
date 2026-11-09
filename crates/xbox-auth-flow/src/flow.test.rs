use super::*;

#[test]
fn refresh_responses_rotate_credentials_or_preserve_the_existing_refresh_grant() {
    for incoming in [None, Some(""), Some(" "), Some("rotated-refresh")] {
        let mut value = json!({"token_type":"bearer", "expires_in":3600, "scope":"scope", "access_token":"new-access"});
        if let Some(token) = incoming {
            value["refresh_token"] = json!(token);
        }
        let output =
            preserve_refresh_token(serde_json::from_value(value).unwrap(), "previous-refresh");
        assert_eq!(output.access_token, "new-access");
        assert_eq!(
            output.refresh_token,
            if incoming == Some("rotated-refresh") {
                "rotated-refresh"
            } else {
                "previous-refresh"
            }
        );
    }
}

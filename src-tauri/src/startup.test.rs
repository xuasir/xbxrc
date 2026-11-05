use super::*;
use std::cell::RefCell;

#[test]
fn failed_service_setup_reports_then_requests_exit_without_returning_setup_error() {
    let events = RefCell::new(Vec::new());
    let result = finish_service_setup(
        Err(error::AppError::Internal("setup-fixture".to_string())),
        |message| events.borrow_mut().push(format!("report:{message}")),
        |code| events.borrow_mut().push(format!("exit:{code}")),
    );
    assert!(result.is_ok());
    let events = events.into_inner();
    assert_eq!(events.len(), 2);
    assert!(events[0].contains("setup-fixture"));
    assert_eq!(events[1], "exit:1");
}

#[test]
fn successful_service_setup_completes_without_failure_feedback_or_exit() {
    assert!(finish_service_setup(
        Ok(()),
        |_| panic!("unexpected failure feedback"),
        |_| panic!("unexpected exit")
    )
    .is_ok());
}

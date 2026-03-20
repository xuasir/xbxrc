use super::recovery_driver::RecoveryDriver;
use super::recovery_scheduler::{RecoverySchedulerDispatch, RecoverySchedulerInput};

pub(super) struct MediaSessionScheduler {
    recovery_driver: RecoveryDriver,
}

impl MediaSessionScheduler {
    pub(super) fn new(recovery_driver: RecoveryDriver) -> Self {
        Self { recovery_driver }
    }

    pub(super) async fn handle_input(&mut self, input: RecoverySchedulerInput) {
        for dispatch in self.recovery_driver.schedule_input(input) {
            self.handle_dispatch(dispatch).await;
        }
    }

    async fn handle_dispatch(&mut self, dispatch: RecoverySchedulerDispatch) {
        self.recovery_driver.apply_dispatch(dispatch).await;
    }
}

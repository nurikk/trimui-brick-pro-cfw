use launch_contract::{validate, Catalog, LaunchRequest};
use session_broker::{
    accepted_handle, BrokerError, SessionBrokerClient, SessionHandle, SessionResult,
};

#[derive(Default)]
pub(crate) struct SimulatorSessionAdapter {
    active: Option<(SessionHandle, LaunchRequest)>,
}

impl SessionBrokerClient for SimulatorSessionAdapter {
    fn submit(
        &mut self,
        request: LaunchRequest,
        catalog: &Catalog,
    ) -> Result<SessionHandle, BrokerError> {
        validate(&request, catalog).map_err(|error| BrokerError::new(error.to_string()))?;
        if self.active.is_some() {
            return Err(BrokerError::new("broker is busy"));
        }
        let handle = accepted_handle(&request);
        self.active = Some((handle.clone(), request));
        Ok(handle)
    }

    fn complete(&mut self, exit_code: i32, duration_ms: u64) -> Result<SessionResult, BrokerError> {
        let (_, request) = self
            .active
            .take()
            .ok_or_else(|| BrokerError::new("no-active-session"))?;
        Ok(SessionResult {
            result_type: "SessionResult",
            journey: "simulator".into(),
            accepted: true,
            runner: Some(request.runner.id),
            core: request.core.map(|core| core.id),
            reason: if exit_code == 0 {
                "success"
            } else {
                "nonzero-exit"
            }
            .into(),
            duration_ms,
            restored: true,
            safe_default: false,
            persistence_status: "not-applicable",
            resume_published: false,
            exit_code: Some(exit_code),
            signal: None,
        })
    }
}

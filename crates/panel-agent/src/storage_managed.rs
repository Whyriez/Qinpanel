//! Managed engine control only. Data-plane transfers bypass privileged IPC.
use crate::{AgentError, system::SystemRunner};
use panel_proto::ManagedStorageRequest;

pub const HELPER: &str = "/usr/local/libexec/qonpanel-object-storage";

pub fn execute(
    runner: &dyn SystemRunner,
    request: ManagedStorageRequest,
) -> Result<serde_json::Value, AgentError> {
    let input = serde_json::to_vec(&request)
        .map_err(|_| AgentError::InvalidInput("Operasi storage tidak valid.".into()))?;
    if input.len() > 196608 {
        return Err(AgentError::InvalidInput(
            "Isian storage terlalu besar.".into(),
        ));
    }
    let result = runner.run_command_with_input("python3", &[HELPER], &input)?;
    if !result.status.success() || result.stdout.len() > 8192 {
        return Err(AgentError::Internal(
            "Layanan storage belum tersedia.".into(),
        ));
    }
    serde_json::from_slice(&result.stdout)
        .map_err(|_| AgentError::Internal("Jawaban layanan storage tidak valid.".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::MockSystemRunner;
    #[test]
    fn invokes_only_the_fixed_helper_and_requires_valid_bounded_json() {
        let runner = MockSystemRunner::new();
        let command = format!("python3 {HELPER}");
        runner.set_response(&command, 0, br#"{"configured":false}"#, b"");
        assert_eq!(
            execute(&runner, ManagedStorageRequest::Status {}).expect("status")["configured"],
            false
        );
        assert_eq!(
            runner.get_commands().as_slice(),
            std::slice::from_ref(&command)
        );
        runner.set_response(&command, 0, b"unexpected output", b"");
        assert!(execute(&runner, ManagedStorageRequest::Status {}).is_err());
    }
}

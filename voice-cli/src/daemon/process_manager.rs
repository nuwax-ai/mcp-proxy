use crate::models::Config;
use crate::VoiceCliError;
use std::path::PathBuf;
use tracing::{info, warn};

/// Manages process lifecycle (PID files, process termination, etc.)
pub struct ProcessManager {
    pid_file_path: PathBuf,
}

impl ProcessManager {
    pub fn new(config: &Config) -> Self {
        Self {
            pid_file_path: PathBuf::from(&config.daemon.pid_file),
        }
    }

    /// Save PID to file
    pub fn save_pid(&self, pid: u32) -> crate::Result<()> {
        if let Some(parent) = self.pid_file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.pid_file_path, pid.to_string())?;
        Ok(())
    }

    /// Read PID from file
    pub fn read_pid(&self) -> crate::Result<Option<u32>> {
        if self.pid_file_path.exists() {
            let pid_str = std::fs::read_to_string(&self.pid_file_path)?;
            let pid = pid_str.trim().parse::<u32>()
                .map_err(|_| VoiceCliError::Daemon("Invalid PID in file".to_string()))?;
            Ok(Some(pid))
        } else {
            Ok(None)
        }
    }

    /// Remove PID file
    pub fn cleanup_pid_file(&self) -> crate::Result<()> {
        if self.pid_file_path.exists() {
            std::fs::remove_file(&self.pid_file_path)?;
        }
        Ok(())
    }

    /// Check if a process is running
    pub fn is_process_running(&self, pid: u32) -> bool {
        #[cfg(unix)]
        {
            use libc::kill;
            unsafe {
                kill(pid as i32, 0) == 0
            }
        }
        
        #[cfg(windows)]
        {
            use winapi::um::processthreadsapi::OpenProcess;
            use winapi::um::winnt::PROCESS_QUERY_INFORMATION;
            use winapi::um::handleapi::CloseHandle;
            
            unsafe {
                let handle = OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid);
                if !handle.is_null() {
                    CloseHandle(handle);
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Gracefully terminate a process
    pub fn terminate_process(&self, pid: u32) -> crate::Result<()> {
        info!("Sending termination signal to PID: {}", pid);

        #[cfg(unix)]
        {
            use libc::{kill, SIGTERM};
            unsafe {
                if kill(pid as i32, SIGTERM) == 0 {
                    info!("Sent SIGTERM to PID: {}", pid);
                } else {
                    warn!("Failed to send SIGTERM to PID: {}", pid);
                    return Err(VoiceCliError::Daemon(
                        format!("Failed to terminate process {}", pid)
                    ));
                }
            }
        }
        
        #[cfg(windows)]
        {
            use winapi::um::processthreadsapi::{OpenProcess, TerminateProcess};
            use winapi::um::winnt::PROCESS_TERMINATE;
            use winapi::um::handleapi::CloseHandle;
            
            unsafe {
                let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
                if !handle.is_null() {
                    TerminateProcess(handle, 0);
                    CloseHandle(handle);
                    info!("Terminated process with PID: {}", pid);
                } else {
                    warn!("Failed to open process with PID: {}", pid);
                    return Err(VoiceCliError::Daemon(
                        format!("Failed to open process {}", pid)
                    ));
                }
            }
        }

        Ok(())
    }

    /// Force kill a process (when graceful termination fails)
    pub fn force_kill_process(&self, pid: u32) -> crate::Result<()> {
        warn!("Force killing process: {}", pid);

        #[cfg(unix)]
        {
            use libc::{kill, SIGKILL};
            unsafe {
                if kill(pid as i32, SIGKILL) == 0 {
                    info!("Sent SIGKILL to PID: {}", pid);
                } else {
                    return Err(VoiceCliError::Daemon(
                        format!("Failed to force kill process {}", pid)
                    ));
                }
            }
        }
        
        #[cfg(windows)]
        {
            // On Windows, TerminateProcess is already forceful
            self.terminate_process(pid)?;
        }

        Ok(())
    }
}
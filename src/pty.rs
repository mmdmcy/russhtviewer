use anyhow::{Context, Result, anyhow};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use std::{
    io::{Read, Write},
    sync::{Arc, Mutex},
    thread,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

pub struct PtySession {
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
    master: Mutex<Box<dyn portable_pty::MasterPty + Send>>,
}

pub struct PtyHandles {
    pub session: Arc<PtySession>,
    pub output_rx: UnboundedReceiver<Vec<u8>>,
}

impl PtySession {
    pub fn spawn(cols: u16, rows: u16) -> Result<PtyHandles> {
        let system = NativePtySystem::default();
        let pair = system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("open pty")?;

        let shell = default_shell();
        let mut cmd = CommandBuilder::new(&shell);
        if let Ok(home) = std::env::var("HOME") {
            cmd.cwd(home);
        }
        let child = pair.slave.spawn_command(cmd).context("spawn shell")?;
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .context("clone pty reader")?;
        let writer = pair.master.take_writer().context("take pty writer")?;
        let (tx, rx): (UnboundedSender<Vec<u8>>, UnboundedReceiver<Vec<u8>>) =
            unbounded_channel();

        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(PtyHandles {
            session: Arc::new(Self {
                writer: Mutex::new(writer),
                child: Mutex::new(child),
                master: Mutex::new(pair.master),
            }),
            output_rx: rx,
        })
    }

    pub fn write_input(&self, data: &[u8]) -> Result<()> {
        let mut writer = self.writer.lock().map_err(|_| anyhow!("pty writer lock"))?;
        writer.write_all(data)?;
        writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        let master = self.master.lock().map_err(|_| anyhow!("pty master lock"))?;
        master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    pub fn try_wait(&self) -> Option<u32> {
        let mut child = self.child.lock().ok()?;
        match child.try_wait() {
            Ok(Some(status)) => Some(status.exit_code()),
            _ => None,
        }
    }
}

fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| {
        if cfg!(windows) {
            "powershell.exe".into()
        } else {
            "/bin/bash".into()
        }
    })
}

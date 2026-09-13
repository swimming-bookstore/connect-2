//! One connect-app pipe, owned by an mpsc actor.

use anyhow::{anyhow, Result};
use crate::protocol::In;
use connect2_teleport::user::{AppRw, Cfg};
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, oneshot};

struct Pipe {
    node: String,
    gen: u64,
    sess: AppRw,
    reading: bool,
}

#[derive(Clone)]
pub struct PipeHub {
    tx: mpsc::Sender<PipeOp>,
}

enum PipeOp {
    Close {
        reply: oneshot::Sender<()>,
    },
    Same {
        dst: String,
        reply: oneshot::Sender<bool>,
    },
    Ensure {
        cfg: Cfg,
        dst: String,
        reply: oneshot::Sender<Result<bool>>,
    },
    Write {
        cmd: In,
        reply: oneshot::Sender<Result<()>>,
    },
    TakeStdout {
        reply: oneshot::Sender<
            Option<(
                tokio::io::ReadHalf<russh::ChannelStream<russh::client::Msg>>,
                u64,
            )>,
        >,
    },
    DropIf {
        gen: u64,
    },
    Alive {
        reply: oneshot::Sender<bool>,
    },
    IsGen {
        gen: u64,
        reply: oneshot::Sender<bool>,
    },
}

impl PipeHub {
    pub fn start() -> Self {
        let (tx, mut rx) = mpsc::channel(32);
        tokio::spawn(async move {
            let mut pipe: Option<Pipe> = None;
            let mut gen = 0u64;
            while let Some(op) = rx.recv().await {
                match op {
                    PipeOp::Close { reply } => {
                        if let Some(mut p) = pipe.take() {
                            if let Err(e) = write_child(&mut p.sess.stdin, &In::Close).await {
                                tracing::debug!("pipe close: {e:#}");
                            }
                        }
                        if reply.send(()).is_err() {
                            tracing::debug!("pipe close: dropped");
                        }
                    }
                    PipeOp::Same { dst, reply } => {
                        let ok = pipe.as_ref().is_some_and(|p| p.node == dst);
                        if reply.send(ok).is_err() {
                            tracing::debug!("pipe same: dropped");
                        }
                    }
                    PipeOp::Ensure { cfg, dst, reply } => {
                        let r = async {
                            if pipe.as_ref().is_some_and(|p| p.node == dst && p.reading) {
                                return Ok(false);
                            }
                            if let Some(mut p) = pipe.take() {
                                if let Err(e) = write_child(&mut p.sess.stdin, &In::Close).await {
                                    tracing::debug!("pipe replace: {e:#}");
                                }
                            }
                            let sess = connect2_teleport::user::dial_app(&cfg, &dst).await?;
                            gen += 1;
                            pipe = Some(Pipe {
                                node: dst,
                                gen,
                                sess: sess.into_rw(),
                                reading: false,
                            });
                            Ok(true)
                        }
                        .await;
                        if reply.send(r).is_err() {
                            tracing::debug!("pipe ensure: dropped");
                        }
                    }
                    PipeOp::Write { cmd, reply } => {
                        let r = async {
                            let p = pipe.as_mut().ok_or_else(|| anyhow!("no session"))?;
                            write_child(&mut p.sess.stdin, &cmd).await
                        }
                        .await;
                        if r.is_err() {
                            pipe = None;
                        }
                        if reply.send(r).is_err() {
                            tracing::debug!("pipe write: dropped");
                        }
                    }
                    PipeOp::TakeStdout { reply } => {
                        let out = match pipe.as_mut() {
                            Some(p) => p.sess.stdout.take().map(|s| {
                                p.reading = true;
                                (s, p.gen)
                            }),
                            None => None,
                        };
                        if reply.send(out).is_err() {
                            tracing::debug!("pipe stdout: dropped");
                        }
                    }
                    PipeOp::DropIf { gen: g } => {
                        if pipe.as_ref().is_some_and(|p| p.gen == g) {
                            pipe = None;
                        }
                    }
                    PipeOp::Alive { reply } => {
                        let ok = pipe.as_ref().is_some_and(|p| p.reading);
                        if reply.send(ok).is_err() {
                            tracing::debug!("pipe alive: dropped");
                        }
                    }
                    PipeOp::IsGen { gen: g, reply } => {
                        let ok = pipe.as_ref().is_some_and(|p| p.gen == g);
                        if reply.send(ok).is_err() {
                            tracing::debug!("pipe gen: dropped");
                        }
                    }
                }
            }
        });
        Self { tx }
    }

    pub async fn close(&self) {
        let (reply, rx) = oneshot::channel();
        if self.tx.send(PipeOp::Close { reply }).await.is_err() {
            tracing::debug!("pipe closed");
            return;
        }
        if rx.await.is_err() {
            tracing::debug!("pipe closed");
        }
    }

    pub async fn same_node(&self, dst: &str) -> bool {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(PipeOp::Same {
                dst: dst.into(),
                reply,
            })
            .await
            .is_err()
        {
            return false;
        }
        rx.await.unwrap_or(false)
    }

    pub async fn ensure(&self, cfg: Cfg, dst: String) -> Result<bool> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(PipeOp::Ensure { cfg, dst, reply })
            .await
            .map_err(|_| anyhow!("pipe closed"))?;
        rx.await.map_err(|_| anyhow!("pipe closed"))?
    }

    pub async fn write(&self, cmd: In) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(PipeOp::Write { cmd, reply })
            .await
            .map_err(|_| anyhow!("no session"))?;
        rx.await.map_err(|_| anyhow!("no session"))?
    }

    pub async fn take_stdout(
        &self,
    ) -> Option<(
        tokio::io::ReadHalf<russh::ChannelStream<russh::client::Msg>>,
        u64,
    )> {
        let (reply, rx) = oneshot::channel();
        if self.tx.send(PipeOp::TakeStdout { reply }).await.is_err() {
            return None;
        }
        rx.await.ok().flatten()
    }

    pub async fn drop_if(&self, gen: u64) {
        if self.tx.send(PipeOp::DropIf { gen }).await.is_err() {
            tracing::debug!("pipe closed");
        }
    }

    pub async fn alive(&self) -> bool {
        let (reply, rx) = oneshot::channel();
        if self.tx.send(PipeOp::Alive { reply }).await.is_err() {
            return false;
        }
        rx.await.unwrap_or(false)
    }

    pub async fn is_gen(&self, gen: u64) -> bool {
        let (reply, rx) = oneshot::channel();
        if self.tx.send(PipeOp::IsGen { gen, reply }).await.is_err() {
            return false;
        }
        rx.await.unwrap_or(false)
    }
}

pub async fn write_child(
    stdin: &mut tokio::io::WriteHalf<russh::ChannelStream<russh::client::Msg>>,
    cmd: &In,
) -> Result<()> {
    let mut line = serde_json::to_vec(cmd)?;
    line.push(b'\n');
    stdin.write_all(&line).await?;
    stdin.flush().await?;
    Ok(())
}

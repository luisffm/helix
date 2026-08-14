//! Seed a demo data dir in-process (`scripts/dev-demo.sh`): spaces, chats with
//! backdated activity, and two finished mock runs. Exits after flushing, so the
//! app can take the data-dir lock right after.
//!
//! Usage: cargo run -p helix-engine --example demo_seed -- <data_dir> [extra_project_path...]
use std::sync::Arc;
use std::time::Duration;

use helix_engine::{EngineCore, HarnessId, default_registry};
use helix_rpc::{RpcReply, RpcService, methods};

struct Seed {
  title: &'static str,
  project: &'static str,
  branch: &'static str,
  age_hours: i64,
  run: bool,
}

const SEEDS: &[Seed] = &[
  Seed {
    title: "Native Helix Rust Rewrite",
    project: "helix",
    branch: "helix/main",
    age_hours: 0,
    run: true,
  },
  Seed {
    title: "Rebalance Player Stats Caps",
    project: "soccertcg",
    branch: "helix/rebalance-player-stat-caps",
    age_hours: 2,
    run: true,
  },
  Seed {
    title: "Craft Premium TCG Experience",
    project: "soccertcg",
    branch: "helix/craft-premium-tcg-exp",
    age_hours: 26,
    run: false,
  },
  Seed {
    title: "Initial Context Exploration",
    project: "helix",
    branch: "helix/initial-context-exploration",
    age_hours: 14,
    run: false,
  },
  Seed {
    title: "Soccer TCG Repo Creation",
    project: "aether",
    branch: "aether/main",
    age_hours: 48,
    run: false,
  },
];

const RUN_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  let data_dir = std::env::args()
    .nth(1)
    .ok_or_else(|| anyhow::anyhow!("usage: demo_seed <data_dir>"))?;
  let home = std::env::var("HOME")?;
  let extra: Vec<String> = std::env::args().skip(2).collect();

  let core = EngineCore::assemble(
    std::path::Path::new(&data_dir),
    Arc::new(default_registry()),
    HarnessId::Mock,
  )?;
  let rpc = core.rpc_service();
  let call = |method: &'static str, params: serde_json::Value| {
    let rpc = rpc.clone();
    async move {
      match rpc.handle(method, params).await {
        Ok(RpcReply::Value(value)) => Ok(value),
        Ok(RpcReply::Stream(_)) => Err(anyhow::anyhow!("{method} answered with a stream")),
        Err(err) => Err(anyhow::anyhow!("{method}: {err}")),
      }
    }
  };

  let device = call(methods::LOCAL_DEVICE, serde_json::json!({}))
    .await?
    .get("deviceId")
    .and_then(|v| v.as_str())
    .ok_or_else(|| anyhow::anyhow!("LocalDevice answered without a deviceId"))?
    .to_string();

  for path in &extra {
    let space_id = uuid::Uuid::new_v4().to_string();
    call(
      methods::MUTATE,
      serde_json::json!({
        "op": "createSpace",
        "spaceId": space_id,
        "deviceId": device,
        "path": path,
        "gitDetected": true,
      }),
    )
    .await?;
    println!("seeded project {path}");
  }

  let mut spaces: Vec<(&str, String)> = Vec::new();
  for project in ["helix", "soccertcg", "helix", "aether"] {
    let space_id = uuid::Uuid::new_v4().to_string();
    call(
      methods::MUTATE,
      serde_json::json!({
          "op": "createSpace",
          "spaceId": space_id,
          "deviceId": device,
          "path": format!("{home}/github/{project}"),
      }),
    )
    .await?;
    spaces.push((project, space_id));
  }
  let space_for = |project: &str| -> String {
    spaces
      .iter()
      .find(|(name, _)| *name == project)
      .map(|(_, id)| id.clone())
      .expect("every seed project has a space")
  };

  for seed in SEEDS {
    let chat_id = uuid::Uuid::new_v4().to_string();
    call(
      methods::MUTATE,
      serde_json::json!({
          "op": "createChat",
          "chatId": chat_id,
          "spaceId": space_for(seed.project),
          "config": {
              "harness": "mock",
              "model": "fable-5",
              "reasoning": null,
              "sandbox": "workspace-write",
          },
      }),
    )
    .await?;
    call(
      methods::MUTATE,
      serde_json::json!({ "op": "renameChat", "chatId": chat_id, "title": seed.title }),
    )
    .await?;
    call(
      methods::MUTATE,
      serde_json::json!({ "op": "setChatBranch", "chatId": chat_id, "branch": seed.branch }),
    )
    .await?;
    if seed.run {
      call(
        methods::QUEUE_COMMAND,
        serde_json::json!({
            "chatId": chat_id,
            "command": {
                "kind": "run",
                "messageId": uuid::Uuid::new_v4().to_string(),
                "request": {
                    "prompt": "Walk me through the streaming pipeline",
                    "model": null,
                    "reasoning": null,
                    "modelOptions": {},
                    "cwd": "/tmp",
                    "sandbox": "workspace-write",
                    "autoApprove": true,
                    "resume": null,
                },
            },
        }),
      )
      .await?;
      wait_for_idle(&core).await;
    }
    let last_message_at = (chrono::Utc::now().timestamp() - seed.age_hours * 3600).max(0) * 1000;
    call(
      methods::MUTATE,
      serde_json::json!({
          "op": "setChatActivity",
          "chatId": chat_id,
          "lastMessageAt": last_message_at,
      }),
    )
    .await?;
    println!("seeded {}", seed.title);
  }

  core.shutdown().await;
  Ok(())
}

async fn wait_for_idle(core: &EngineCore) {
  tokio::time::sleep(Duration::from_millis(200)).await;
  let deadline = tokio::time::Instant::now() + RUN_TIMEOUT;
  while core.sessions.any_active() && tokio::time::Instant::now() < deadline {
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
}

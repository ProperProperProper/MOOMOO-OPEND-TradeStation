use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use crate::validate_loopback_only;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenDFunds {
    pub acc_id: String,
    pub trd_env: i32,
    pub total_assets: f64,
    pub cash: f64,
    pub power: f64,
    pub market_val: f64,
    pub currency: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenDPosition {
    pub position_id: String,
    pub symbol: String,
    pub name: String,
    pub qty: f64,
    pub can_sell_qty: f64,
    pub price: f64,
    pub cost_price: f64,
    pub val: f64,
    pub pl_val: f64,
}

pub struct OpenDProtoSession {
    stream: TcpStream,
    serial_no: u32,
}

impl OpenDProtoSession {
    pub async fn connect(raw_addr: &str) -> Result<Self> {
        let trimmed = raw_addr.trim();
        let target = trimmed
            .strip_prefix("http://")
            .or_else(|| trimmed.strip_prefix("https://"))
            .unwrap_or(trimmed);

        validate_loopback_only(target)?;

        let stream = tokio::time::timeout(
            std::time::Duration::from_millis(2000),
            TcpStream::connect(target),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Connection timeout to OpenD at {target}"))??;

        let mut session = Self { stream, serial_no: 1 };
        session.init_connect().await?;
        Ok(session)
    }

    async fn init_connect(&mut self) -> Result<()> {
        let body = serde_json::json!({
            "c2s": {
                "clientVer": 300,
                "clientID": "TradeStation",
                "recvNotify": true
            }
        });
        let rsp = self.query(1001, &body).await?;
        let ret_type = rsp.get("retType").and_then(|v| v.as_i64()).unwrap_or(-1);
        if ret_type != 0 {
            let msg = rsp.get("retMsg").and_then(|v| v.as_str()).unwrap_or("unknown");
            bail!("InitConnect failed: {msg}");
        }
        Ok(())
    }

    pub async fn query(&mut self, proto_id: u32, body: &serde_json::Value) -> Result<serde_json::Value> {
        let body_bytes = serde_json::to_vec(body)?;
        let mut hasher = Sha1::new();
        hasher.update(&body_bytes);
        let sha1_bytes: [u8; 20] = hasher.finalize().into();

        let mut header = [0u8; 44];
        header[0] = b'F';
        header[1] = b'T';
        header[2..6].copy_from_slice(&proto_id.to_le_bytes());
        header[6] = 1; // JSON format
        header[7] = 0; // proto_ver
        header[8..12].copy_from_slice(&self.serial_no.to_le_bytes());
        header[12..16].copy_from_slice(&(body_bytes.len() as u32).to_le_bytes());
        header[16..36].copy_from_slice(&sha1_bytes);
        self.serial_no += 1;

        tokio::time::timeout(std::time::Duration::from_millis(2000), async {
            self.stream.write_all(&header).await?;
            self.stream.write_all(&body_bytes).await?;
            self.stream.flush().await?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .map_err(|_| anyhow::anyhow!("Send timeout to OpenD"))??;

        let mut rsp_head = [0u8; 44];
        tokio::time::timeout(std::time::Duration::from_millis(3000), self.stream.read_exact(&mut rsp_head))
            .await
            .map_err(|_| anyhow::anyhow!("Read header timeout from OpenD"))??;

        if rsp_head[0] != b'F' || rsp_head[1] != b'T' {
            bail!("Invalid header signature from OpenD: expected 'FT'");
        }

        let body_len = u32::from_le_bytes([rsp_head[12], rsp_head[13], rsp_head[14], rsp_head[15]]) as usize;
        let mut rsp_body = vec![0u8; body_len];
        tokio::time::timeout(std::time::Duration::from_millis(3000), self.stream.read_exact(&mut rsp_body))
            .await
            .map_err(|_| anyhow::anyhow!("Read body timeout from OpenD"))??;

        let res: serde_json::Value = serde_json::from_slice(&rsp_body)
            .context("Parsing JSON response from OpenD")?;
        Ok(res)
    }

    /// Fetches all paper account IDs (trdEnv = 0). Returns vector of (acc_id, trd_market).
    pub async fn get_paper_accounts(&mut self) -> Result<Vec<(u64, i32)>> {
        let req = serde_json::json!({
            "c2s": { "userID": 0 }
        });
        let res = self.query(2001, &req).await?;
        let mut out = Vec::new();
        if let Some(list) = res.pointer("/s2c/accList").and_then(|v| v.as_array()) {
            for item in list {
                let env = item.get("trdEnv").and_then(|v| v.as_i64()).unwrap_or(-1);
                if env == 0 { // paper / simulate
                    let acc_id_val = item.get("accID")
                        .and_then(|v| v.as_str().and_then(|s| s.parse::<u64>().ok()).or_else(|| v.as_u64()));
                    let market = item.pointer("/trdMarketAuthList/0")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(2) as i32;
                    if let Some(id) = acc_id_val {
                        out.push((id, market));
                    }
                }
            }
        }
        Ok(out)
    }

    /// Fetches paper trading funds for a specific account.
    pub async fn get_funds(&mut self, acc_id: u64, trd_market: i32) -> Result<OpenDFunds> {
        let req = serde_json::json!({
            "c2s": {
                "header": {
                    "trdEnv": 0,
                    "accID": acc_id,
                    "trdMarket": trd_market
                },
                "refreshCache": true
            }
        });
        let res = self.query(2101, &req).await?;
        let funds = res.pointer("/s2c/funds")
            .ok_or_else(|| anyhow::anyhow!("Missing funds in OpenD response: {res}"))?;

        let total_assets = funds.get("totalAssets").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let cash = funds.get("cash").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let power = funds.get("power").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let market_val = funds.get("marketVal").and_then(|v| v.as_f64()).unwrap_or(0.0);

        Ok(OpenDFunds {
            acc_id: acc_id.to_string(),
            trd_env: 0,
            total_assets,
            cash,
            power,
            market_val,
            currency: "USD".to_string(),
        })
    }

    /// Fetches positions for a specific paper account.
    pub async fn get_positions(&mut self, acc_id: u64, trd_market: i32) -> Result<Vec<OpenDPosition>> {
        let req = serde_json::json!({
            "c2s": {
                "header": {
                    "trdEnv": 0,
                    "accID": acc_id,
                    "trdMarket": trd_market
                },
                "refreshCache": true
            }
        });
        let res = self.query(2102, &req).await?;
        let mut out = Vec::new();
        if let Some(positions) = res.pointer("/s2c/positionList").and_then(|v| v.as_array()) {
            for p in positions {
                let qty = p.get("qty").and_then(|v| v.as_f64()).unwrap_or(0.0);
                if qty > 0.0 {
                    let code = p.get("code").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let price = p.get("price").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let cost_price = p.get("costPrice").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let val = p.get("val").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let pl_val = p.get("plVal").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let position_id = p.get("positionID")
                        .and_then(|v| v.as_str().map(|s| s.to_string()).or_else(|| v.as_i64().map(|n| n.to_string())))
                        .unwrap_or_default();
                    let can_sell = p.get("canSellQty").and_then(|v| v.as_f64()).unwrap_or(qty);
                    out.push(OpenDPosition {
                        position_id,
                        symbol: code,
                        name,
                        qty,
                        can_sell_qty: can_sell,
                        price,
                        cost_price,
                        val,
                        pl_val,
                    });
                }
            }
        }
        Ok(out)
    }
}

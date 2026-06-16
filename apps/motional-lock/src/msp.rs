use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
pub const EVENT_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorDescription {
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorState {
    pub name: String,
    pub triggered: Option<bool>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub last_triggered_at: Option<String>,
    #[serde(default)]
    pub seconds_since_triggered: Option<u64>,
    #[serde(default)]
    pub observed_at: Option<String>,
    #[serde(default)]
    pub sequence: Option<u64>,
    #[serde(default)]
    pub raw: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SensorError {
    pub name: String,
    pub code: String,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug)]
pub enum MspEvent {
    StateChanged {
        subscription_id: String,
        state: SensorState,
    },
    ResyncRequired {
        subscription_id: String,
    },
    Other(Value),
}

pub struct MspConnection {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
    next_id: u64,
}

impl MspConnection {
    pub fn connect(address: &str, token: Option<&str>, client_name: &str) -> Result<Self> {
        let stream = connect_tcp(address)?;
        stream
            .set_read_timeout(Some(DEFAULT_TIMEOUT))
            .context("failed to set MSP read timeout")?;
        stream
            .set_write_timeout(Some(DEFAULT_TIMEOUT))
            .context("failed to set MSP write timeout")?;

        let reader = BufReader::new(stream.try_clone().context("failed to clone TCP stream")?);
        let mut connection = Self {
            writer: stream,
            reader,
            next_id: 1,
        };

        let hello_id = connection.next_request_id();
        connection.send(json!({
            "id": hello_id,
            "type": "hello",
            "version": 1,
            "client": {
                "name": client_name,
                "version": env!("CARGO_PKG_VERSION")
            }
        }))?;

        let hello = connection.read_value()?;
        ensure_response_id(&hello, &hello_id)?;
        ensure_type(&hello, "server_hello")?;
        if hello.get("version").and_then(Value::as_u64) != Some(1) {
            bail!("MSP server does not support protocol version 1");
        }

        let auth_required = hello
            .get("auth_required")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if auth_required {
            let token = token
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| anyhow!("MSP server requires a bearer token"))?;
            connection.authenticate(token)?;
        }

        Ok(connection)
    }

    pub fn list_sensors(&mut self) -> Result<Vec<SensorDescription>> {
        let id = self.next_request_id();
        self.send(json!({
            "id": id,
            "type": "list"
        }))?;
        let response = self.read_response(&id)?;
        ensure_type(&response, "sensor_list")?;

        serde_json::from_value(response["sensors"].clone()).context("invalid sensor_list response")
    }

    pub fn get_states(&mut self, sensors: &[String]) -> Result<Vec<SensorState>> {
        let id = self.next_request_id();
        self.send(json!({
            "id": id,
            "type": "get",
            "sensors": sensors
        }))?;
        let response = self.read_response(&id)?;
        ensure_type(&response, "state")?;
        report_sensor_errors(&response)?;

        serde_json::from_value(response["states"].clone()).context("invalid state response")
    }

    pub fn subscribe(&mut self, sensors: &[String], send_initial: bool) -> Result<String> {
        let id = self.next_request_id();
        self.send(json!({
            "id": id,
            "type": "subscribe",
            "sensors": sensors,
            "send_initial": send_initial
        }))?;
        let response = self.read_response(&id)?;
        ensure_type(&response, "subscribed")?;

        let subscription_id = response
            .get("subscription_id")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .ok_or_else(|| anyhow!("subscribed response missing subscription_id"))?;

        self.reader
            .get_ref()
            .set_read_timeout(Some(EVENT_IDLE_TIMEOUT))
            .context("failed to set MSP event read timeout")?;

        Ok(subscription_id)
    }

    pub fn ping(&mut self) -> Result<()> {
        let id = self.next_request_id();
        self.send(json!({
            "id": id,
            "type": "ping"
        }))?;
        let response = self.read_response(&id)?;
        ensure_type(&response, "pong")
    }

    pub fn read_event(&mut self) -> Result<MspEvent> {
        let value = self.read_value()?;
        match value.get("type").and_then(Value::as_str) {
            Some("event") => parse_event(value),
            Some("error") => Err(error_from_value(&value)),
            _ => Ok(MspEvent::Other(value)),
        }
    }

    fn authenticate(&mut self, token: &str) -> Result<()> {
        let id = self.next_request_id();
        self.send(json!({
            "id": id,
            "type": "auth",
            "scheme": "bearer",
            "token": token
        }))?;
        let response = self.read_response(&id)?;
        ensure_type(&response, "ok")
    }

    fn read_response(&mut self, id: &str) -> Result<Value> {
        loop {
            let value = self.read_value()?;
            if value.get("id").and_then(Value::as_str) == Some(id) {
                if value.get("type").and_then(Value::as_str) == Some("error") {
                    return Err(error_from_value(&value));
                }
                return Ok(value);
            }
        }
    }

    fn next_request_id(&mut self) -> String {
        let id = self.next_id;
        self.next_id += 1;
        id.to_string()
    }

    fn send(&mut self, value: Value) -> Result<()> {
        serde_json::to_writer(&mut self.writer, &value).context("failed to encode MSP message")?;
        self.writer
            .write_all(b"\n")
            .context("failed to write MSP message newline")?;
        self.writer.flush().context("failed to flush MSP message")
    }

    fn read_value(&mut self) -> Result<Value> {
        let mut line = String::new();
        let bytes = match self.reader.read_line(&mut line) {
            Ok(bytes) => bytes,
            Err(error) if matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {
                bail!("MSP read timed out")
            }
            Err(error) => return Err(error).context("failed to read MSP message"),
        };
        if bytes == 0 {
            bail!("MSP server closed the connection");
        }
        if bytes > 65_536 {
            bail!("MSP message exceeded 65536 bytes");
        }
        serde_json::from_str(line.trim_end()).context("failed to decode MSP JSON message")
    }
}

fn connect_tcp(address: &str) -> Result<TcpStream> {
    let mut last_error = None;
    for address in address
        .to_socket_addrs()
        .with_context(|| format!("failed to resolve {address}"))?
    {
        match TcpStream::connect_timeout(&address, DEFAULT_TIMEOUT) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }

    match last_error {
        Some(error) if error.kind() == ErrorKind::TimedOut => {
            Err(anyhow!("connection timed out after {:?}", DEFAULT_TIMEOUT))
                .with_context(|| format!("failed to connect to MSP server {address}"))
        }
        Some(error) => Err(anyhow::Error::from(error))
            .with_context(|| format!("failed to connect to MSP server {address}")),
        None => Err(anyhow!("no socket addresses resolved"))
            .with_context(|| format!("failed to connect to MSP server {address}")),
    }
}

fn ensure_response_id(value: &Value, id: &str) -> Result<()> {
    if value.get("id").and_then(Value::as_str) == Some(id) {
        Ok(())
    } else {
        bail!("MSP response id mismatch")
    }
}

fn ensure_type(value: &Value, expected: &str) -> Result<()> {
    match value.get("type").and_then(Value::as_str) {
        Some(actual) if actual == expected => Ok(()),
        Some("error") => Err(error_from_value(value)),
        Some(actual) => bail!("expected MSP message type {expected}, got {actual}"),
        None => bail!("MSP message missing type"),
    }
}

fn error_from_value(value: &Value) -> anyhow::Error {
    let code = value
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("unknown_error");
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("MSP request failed");
    anyhow!("{code}: {message}")
}

fn report_sensor_errors(value: &Value) -> Result<()> {
    let Some(errors) = value.get("errors") else {
        return Ok(());
    };
    let errors: Vec<SensorError> =
        serde_json::from_value(errors.clone()).context("invalid state errors response")?;
    if errors.is_empty() {
        return Ok(());
    }

    let joined = errors
        .into_iter()
        .map(|error| {
            format!(
                "{}: {}{}",
                error.name,
                error.code,
                error
                    .message
                    .map(|message| format!(" ({message})"))
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    bail!("MSP sensor errors: {joined}")
}

fn parse_event(value: Value) -> Result<MspEvent> {
    let subscription_id = value
        .get("subscription_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    match value.get("event").and_then(Value::as_str) {
        Some("state_changed") => {
            let state = serde_json::from_value(value["state"].clone())
                .context("invalid state_changed event state")?;
            Ok(MspEvent::StateChanged {
                subscription_id,
                state,
            })
        }
        Some("resync_required") => Ok(MspEvent::ResyncRequired { subscription_id }),
        _ => Ok(MspEvent::Other(value)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_state_changed_event() {
        let value = json!({
            "type": "event",
            "subscription_id": "sub-1",
            "event": "state_changed",
            "state": {
                "name": "office",
                "triggered": true,
                "status": "ok",
                "observed_at": "2026-06-15T14:06:01.000Z"
            }
        });

        let event = parse_event(value).unwrap();
        match event {
            MspEvent::StateChanged {
                subscription_id,
                state,
            } => {
                assert_eq!(subscription_id, "sub-1");
                assert_eq!(state.name, "office");
                assert_eq!(state.triggered, Some(true));
            }
            _ => panic!("wrong event type"),
        }
    }
}

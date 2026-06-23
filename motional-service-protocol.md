# Motional Service Protocol

Version: 1

Motional Service Protocol, or MSP, is a small TCP protocol for reading and subscribing to motion-like sensor state. It is intended for local automation networks where a service can collect sensor data from Hubitat, Home Assistant, MQTT, hardware GPIO, or any other source, then expose one uniform interface to clients.

## Goals

- Let any number of clients connect concurrently.
- Let clients list available sensors.
- Let clients poll one or more sensor states.
- Let clients subscribe to sensor state changes.
- Keep the protocol simple to implement on Linux, macOS, and Windows.
- Avoid exposing a home automation controller's native UI or API ports to broad network segments.

## Transport

MSP runs over TCP. The default port is `7080`.

All protocol bytes are UTF-8 encoded text. Every message is one JSON object followed by a single line feed byte, `\n` (`0x0a`). Receivers MUST also accept `\r\n`.

This format is commonly called JSON Lines or NDJSON.

```text
{"id":"1","type":"hello","version":1}
{"id":"2","type":"list"}
```

Implementations MUST NOT emit a byte order mark. Messages MUST be valid UTF-8. A peer receiving invalid UTF-8 MAY close the connection.

## Message Size

The maximum message size is `65536` bytes, including the trailing newline. A server MAY advertise a different maximum in `server_hello.max_message_bytes`.

If a peer receives a larger message, it SHOULD send an error with code `message_too_large` and close the connection.

## JSON Rules

- Every message MUST be a JSON object.
- Every message MUST contain a string `type`.
- Client requests SHOULD contain a string `id`.
- Server responses to requests MUST contain the same `id`.
- Request IDs are scoped to one TCP connection.
- Unknown object members MUST be ignored.
- Unknown message types MUST receive an `error` response.
- Timestamps MUST be RFC 3339 UTC strings, for example `2026-06-15T14:05:33.120Z`.
- Durations and ages are expressed in integer seconds unless the field name explicitly says otherwise.

## Connection Lifecycle

After TCP connection, the client sends `hello`.

```json
{"id":"1","type":"hello","version":1,"client":{"name":"motional-lock","version":"0.1.0"}}
```

The server replies with `server_hello`.

```json
{"id":"1","type":"server_hello","version":1,"server":{"name":"motional-service","version":"0.1.0"},"auth_required":true,"max_message_bytes":65536,"heartbeat_seconds":30}
```

If `auth_required` is true, the client MUST authenticate before issuing `list`, `get`, or `subscribe`.

```json
{"id":"2","type":"auth","scheme":"bearer","token":"msp_abc123"}
```

Successful authentication:

```json
{"id":"2","type":"ok"}
```

Failed authentication:

```json
{"id":"2","type":"error","code":"unauthorized","message":"invalid token"}
```

Servers SHOULD close the connection after repeated authentication failures.

## Sensor Model

A sensor has a stable string `name`. The name is the protocol identifier used in `get` and `subscribe`.

Sensor names:

- MUST be unique within one server.
- MUST be between 1 and 128 UTF-8 bytes.
- SHOULD use `a-z`, `0-9`, `.`, `_`, and `-`.
- MUST NOT contain `/`, control characters, quotes, or whitespace.

A sensor description has this shape:

```json
{
  "name": "office",
  "display_name": "Office Motion",
  "kind": "motion",
  "capabilities": ["triggered"],
  "metadata": {
    "room": "office",
    "source": "hubitat"
  }
}
```

`kind` SHOULD be one of:

- `motion`
- `presence`
- `occupancy`
- `contact`
- `button`
- `virtual`
- `other`

The `metadata` object is optional. Clients MUST NOT require any metadata field.

## Sensor State

Sensor state has this shape:

```json
{
  "name": "office",
  "triggered": false,
  "status": "ok",
  "last_triggered_at": "2026-06-15T14:00:33.120Z",
  "seconds_since_triggered": 300,
  "observed_at": "2026-06-15T14:05:33.120Z",
  "sequence": 1842,
  "raw": {
    "motion": "inactive"
  }
}
```

Fields:

- `name`: sensor protocol name.
- `triggered`: `true`, `false`, or `null`. `null` means the server cannot currently determine the logical triggered state.
- `status`: `ok`, `unknown`, `unavailable`, or `error`.
- `last_triggered_at`: last time the sensor was known to be triggered, or `null`.
- `seconds_since_triggered`: seconds between `observed_at` and `last_triggered_at`, or `null`.
- `observed_at`: time the server observed or computed this state.
- `sequence`: unsigned integer that increases when this sensor's state changes. Servers SHOULD start at `1` after process start. Clients MUST treat it as opaque except for ordering within one sensor on one connection.
- `raw`: optional source-specific object.

The normalized meaning of `triggered` is intentionally broad:

- Motion active: `triggered: true`
- Presence present: `triggered: true`
- Occupancy occupied: `triggered: true`
- Motion inactive, presence not present, or occupancy clear: `triggered: false`

## Commands

### `list`

Lists sensors visible to the authenticated client.

Request:

```json
{"id":"3","type":"list"}
```

Response:

```json
{
  "id": "3",
  "type": "sensor_list",
  "sensors": [
    {
      "name": "office",
      "display_name": "Office Motion",
      "kind": "motion",
      "capabilities": ["triggered"]
    }
  ]
}
```

### `get`

Gets current state for one or more sensors.

Request:

```json
{"id":"4","type":"get","sensors":["office","kitchen"]}
```

The special value `"*"` means every visible sensor.

```json
{"id":"4","type":"get","sensors":["*"]}
```

Response:

```json
{
  "id": "4",
  "type": "state",
  "states": [
    {
      "name": "office",
      "triggered": false,
      "status": "ok",
      "last_triggered_at": "2026-06-15T14:00:33.120Z",
      "seconds_since_triggered": 300,
      "observed_at": "2026-06-15T14:05:33.120Z",
      "sequence": 1842
    }
  ]
}
```

If some requested sensors do not exist or are not visible, the server SHOULD return states for the valid sensors and include per-sensor errors.

```json
{
  "id": "4",
  "type": "state",
  "states": [],
  "errors": [
    {"name":"garage","code":"not_found","message":"sensor not found"}
  ]
}
```

### `subscribe`

Subscribes the connection to state change events.

Request:

```json
{"id":"5","type":"subscribe","sensors":["office","kitchen"]}
```

The special value `"*"` subscribes to every visible sensor.

Optional `send_initial` controls whether the server immediately sends current state events after confirming the subscription. The default is `true`.

```json
{"id":"5","type":"subscribe","sensors":["*"],"send_initial":true}
```

Response:

```json
{"id":"5","type":"subscribed","subscription_id":"sub-1","sensors":["office","kitchen"]}
```

After this response, the server sends asynchronous `event` messages for matching state changes.

```json
{
  "type": "event",
  "subscription_id": "sub-1",
  "event": "state_changed",
  "state": {
    "name": "office",
    "triggered": true,
    "status": "ok",
    "last_triggered_at": "2026-06-15T14:06:01.000Z",
    "seconds_since_triggered": 0,
    "observed_at": "2026-06-15T14:06:01.000Z",
    "sequence": 1843
  }
}
```

Servers MAY coalesce rapid source updates, but MUST preserve final state order per sensor.

### `unsubscribe`

Cancels one subscription.

Request:

```json
{"id":"6","type":"unsubscribe","subscription_id":"sub-1"}
```

Response:

```json
{"id":"6","type":"ok"}
```

### `ping`

Checks liveness.

Request:

```json
{"id":"7","type":"ping"}
```

Response:

```json
{"id":"7","type":"pong","time":"2026-06-15T14:06:10.000Z"}
```

Clients SHOULD send `ping` if they have not received any message for `server_hello.heartbeat_seconds`. Servers MAY close idle connections.

### `bye`

Gracefully closes the connection.

Request:

```json
{"id":"8","type":"bye"}
```

Response:

```json
{"id":"8","type":"ok"}
```

After sending `ok`, the server SHOULD close the TCP connection.

## Errors

Error response:

```json
{"id":"4","type":"error","code":"not_authenticated","message":"authenticate first"}
```

`id` is omitted only when the error cannot be associated with a request.

Standard error codes:

- `bad_json`: message is not valid JSON.
- `bad_request`: request is syntactically valid JSON but invalid MSP.
- `unsupported_version`: protocol version is unsupported.
- `unknown_type`: message type is unknown.
- `not_authenticated`: authentication is required first.
- `unauthorized`: authentication failed or token lacks access.
- `not_found`: sensor or subscription does not exist.
- `message_too_large`: message exceeded maximum size.
- `rate_limited`: client sent too many requests.
- `internal_error`: server failed unexpectedly.
- `unavailable`: upstream sensor source is unavailable.

Servers SHOULD keep the connection open after ordinary request errors. Servers MAY close the connection after `bad_json`, `message_too_large`, repeated `rate_limited`, or repeated authentication failures.

## Authorization

MSP defines bearer-token authentication but does not define token storage, issuance, or policy.

Servers SHOULD support per-token sensor visibility. `list`, `get`, and `subscribe` MUST only expose sensors authorized for the authenticated client.

Clients MUST NOT send commands before successful `auth` when `auth_required` is true.

Deployments SHOULD avoid sending MSP bearer tokens over untrusted networks without an encrypted transport such as WireGuard, TLS, SSH tunneling, or an equivalent trusted overlay.

Clients SHOULD fail closed before sending bearer tokens over plaintext TCP to non-loopback addresses unless the user explicitly enables insecure remote MSP token authentication.

## Concurrency

Servers MUST support multiple simultaneous TCP clients.

Within one connection:

- Clients MAY send multiple requests without waiting for earlier responses.
- Servers MAY respond out of order.
- Responses are correlated by `id`.
- Asynchronous `event` messages have no request `id`.

Clients that do not implement pipelining can send one request at a time.

## Backpressure

If a client cannot read events quickly enough, the server MAY:

- Buffer a bounded number of events.
- Coalesce pending events by sensor.
- Drop the connection.

If events are dropped or coalesced, the server SHOULD send:

```json
{"type":"event","subscription_id":"sub-1","event":"resync_required"}
```

After `resync_required`, the client SHOULD issue `get` for its subscribed sensors.

## Example Session

Client:

```json
{"id":"1","type":"hello","version":1,"client":{"name":"motional-lock","version":"0.1.0"}}
```

Server:

```json
{"id":"1","type":"server_hello","version":1,"server":{"name":"motional-service","version":"0.1.0"},"auth_required":true,"max_message_bytes":65536,"heartbeat_seconds":30}
```

Client:

```json
{"id":"2","type":"auth","scheme":"bearer","token":"msp_abc123"}
```

Server:

```json
{"id":"2","type":"ok"}
```

Client:

```json
{"id":"3","type":"list"}
```

Server:

```json
{"id":"3","type":"sensor_list","sensors":[{"name":"office","display_name":"Office Motion","kind":"motion","capabilities":["triggered"]}]}
```

Client:

```json
{"id":"4","type":"subscribe","sensors":["office"],"send_initial":true}
```

Server:

```json
{"id":"4","type":"subscribed","subscription_id":"sub-1","sensors":["office"]}
{"type":"event","subscription_id":"sub-1","event":"state_changed","state":{"name":"office","triggered":false,"status":"ok","last_triggered_at":"2026-06-15T14:00:33.120Z","seconds_since_triggered":300,"observed_at":"2026-06-15T14:05:33.120Z","sequence":1842}}
```

Later, when motion is detected:

```json
{"type":"event","subscription_id":"sub-1","event":"state_changed","state":{"name":"office","triggered":true,"status":"ok","last_triggered_at":"2026-06-15T14:06:01.000Z","seconds_since_triggered":0,"observed_at":"2026-06-15T14:06:01.000Z","sequence":1843}}
```

## Minimal Client Behavior

A minimal polling client:

1. Open TCP connection to host port `7080`.
2. Send `hello`.
3. Send `auth` if required.
4. Send `get` for one sensor.
5. Read `state`.
6. Repeat `get` as needed.

A minimal subscribing client:

1. Open TCP connection to host port `7080`.
2. Send `hello`.
3. Send `auth` if required.
4. Send `subscribe`.
5. Process `event` messages until disconnected.
6. Reconnect and resubscribe after disconnect.

## Minimal Server Behavior

A minimal compliant server:

1. Listens on TCP port `7080`.
2. Accepts multiple client connections.
3. Parses newline-delimited UTF-8 JSON objects.
4. Implements `hello`, `auth` if configured, `list`, `get`, `subscribe`, `unsubscribe`, `ping`, and `bye`.
5. Maintains normalized state for every visible sensor.
6. Sends `event` messages when subscribed sensor state changes.
7. Enforces token authorization before revealing sensor names or states.

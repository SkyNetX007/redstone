// redstone-core/src/slp.rs
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[derive(Debug, Clone)]
pub struct ServerStatus {
    pub version: String,
    pub protocol: i32,
    pub online_players: i32,
    pub max_players: i32,
    pub motd: String,
    pub favicon: Option<String>,
    pub latency_ms: u64,
}

pub async fn ping_server(
    host: &str,
    port: u16,
) -> Result<ServerStatus, Box<dyn std::error::Error>> {
    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        ping_server_inner(host, port),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err("SLP ping timed out (5s)".into()),
    }
}

async fn ping_server_inner(
    host: &str,
    port: u16,
) -> Result<ServerStatus, Box<dyn std::error::Error>> {
    let start = std::time::Instant::now();
    let mut stream = TcpStream::connect((host, port)).await?;

    // Handshake (packet ID 0x00)
    let mut handshake = Vec::new();
    write_varint(&mut handshake, -1);
    write_string(&mut handshake, host);
    handshake.extend_from_slice(&port.to_be_bytes());
    write_varint(&mut handshake, 1);
    send_packet(&mut stream, 0x00, &handshake).await?;

    // Status Request (packet ID 0x00)
    send_packet(&mut stream, 0x00, &[]).await?;

    // Read Response
    let response = recv_packet(&mut stream).await?;
    let (_, raw) = response;
    let json_str = read_string(&raw, &mut 0)?;
    let latency = start.elapsed().as_millis() as u64;

    // Ping (packet ID 0x01) – optional, latency measurement
    let ping_payload = 42i64.to_be_bytes();
    send_packet(&mut stream, 0x01, &ping_payload).await?;
    let _ = recv_packet(&mut stream).await;

    drop(stream);
    parse_slp_response(&json_str, latency)
}

fn parse_slp_response(
    json_str: &str,
    latency_ms: u64,
) -> Result<ServerStatus, Box<dyn std::error::Error>> {
    let v: serde_json::Value = serde_json::from_str(json_str)?;

    let version = v["version"]["name"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let protocol = v["version"]["protocol"].as_i64().unwrap_or(-1) as i32;
    let online_players = v["players"]["online"].as_i64().unwrap_or(0) as i32;
    let max_players = v["players"]["max"].as_i64().unwrap_or(0) as i32;

    let motd = extract_motd(&v["description"]);
    let favicon = v["favicon"].as_str().map(|s| s.to_string());

    Ok(ServerStatus {
        version,
        protocol,
        online_players,
        max_players,
        motd,
        favicon,
        latency_ms,
    })
}

fn extract_motd(desc: &serde_json::Value) -> String {
    match desc {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(_) => desc["text"].as_str().unwrap_or("").to_string(),
        _ => String::new(),
    }
}

// ─── Minecraft Protocol Helpers ───

async fn send_packet(
    stream: &mut TcpStream,
    id: i32,
    data: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut packet = Vec::new();
    write_varint(&mut packet, id);
    packet.extend_from_slice(data);

    let mut final_buf = Vec::new();
    write_varint(&mut final_buf, packet.len() as i32);
    final_buf.extend_from_slice(&packet);

    stream.write_all(&final_buf).await?;
    Ok(())
}

async fn recv_packet(stream: &mut TcpStream) -> Result<(i32, Vec<u8>), Box<dyn std::error::Error>> {
    let len = read_varint_async(stream).await? as usize;
    let mut data = vec![0u8; len];
    stream.read_exact(&mut data).await?;

    let mut pos = 0;
    let id = read_varint(&data, &mut pos)?;
    let rest = data[pos..].to_vec();
    Ok((id, rest))
}

// ─── VarInt / String encoding ───

fn write_varint(buf: &mut Vec<u8>, value: i32) {
    let mut val = value as u32;
    loop {
        if val & !0x7F == 0 {
            buf.push(val as u8);
            return;
        }
        buf.push((val as u8 & 0x7F) | 0x80);
        val >>= 7;
    }
}

fn read_varint(buf: &[u8], pos: &mut usize) -> Result<i32, Box<dyn std::error::Error>> {
    let mut value = 0i32;
    let mut shift = 0;
    loop {
        if *pos >= buf.len() {
            return Err("unexpected end of varint".into());
        }
        let byte = buf[*pos];
        *pos += 1;
        value |= ((byte & 0x7F) as i32) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
        if shift > 31 {
            return Err("varint too long".into());
        }
    }
}

async fn read_varint_async(stream: &mut TcpStream) -> Result<i32, Box<dyn std::error::Error>> {
    let mut value = 0i32;
    let mut shift = 0;
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).await?;
        value |= ((byte[0] & 0x7F) as i32) << shift;
        if byte[0] & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
        if shift > 31 {
            return Err("varint too long".into());
        }
    }
}

fn write_string(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    write_varint(buf, bytes.len() as i32);
    buf.extend_from_slice(bytes);
}

fn read_string(buf: &[u8], pos: &mut usize) -> Result<String, Box<dyn std::error::Error>> {
    let len = read_varint(buf, pos)? as usize;
    if *pos + len > buf.len() {
        return Err("string exceeds buffer".into());
    }
    let s = String::from_utf8(buf[*pos..*pos + len].to_vec())?;
    *pos += len;
    Ok(s)
}

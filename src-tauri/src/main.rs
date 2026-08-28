#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use ssh2::{Session, Sftp};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize, Deserialize, Clone)]
struct FileEntry { name: String, path: String, kind: String, size: u64, modified: u64, extension: String, permissions: Option<String> }
#[derive(Debug, Serialize, Deserialize)]
struct DirectoryListing { path: String, entries: Vec<FileEntry> }
#[derive(Debug, Serialize, Deserialize, Clone)]
struct ConnectionConfig { id: String, name: String, host: String, port: u16, username: String, #[serde(rename = "remotePath")] remote_path: String }
#[derive(Debug, Serialize, Deserialize, Clone)]
struct SearchCriteria { query: String, kind: String, date: String, content: bool }

fn modified_time(metadata: &fs::Metadata) -> u64 { metadata.modified().ok().and_then(|time| time.duration_since(UNIX_EPOCH).ok()).map(|duration| duration.as_millis() as u64).unwrap_or(0) }
fn extension(path: &Path) -> String { path.extension().and_then(|value| value.to_str()).unwrap_or("").to_lowercase() }
fn dirs_home() -> PathBuf { std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/")) }
fn local_entry(path: PathBuf) -> Result<FileEntry, String> { let metadata = fs::metadata(&path).map_err(|error| error.to_string())?; let kind = if metadata.is_dir() { "folder" } else { "file" }.to_string(); Ok(FileEntry { name: path.file_name().and_then(|value| value.to_str()).unwrap_or("").to_string(), path: path.to_string_lossy().to_string(), kind, size: metadata.len(), modified: modified_time(&metadata), extension: extension(&path), permissions: None }) }

#[tauri::command]
fn read_local_directory(path: String) -> Result<DirectoryListing, String> {
    let requested = if path.trim().is_empty() { dirs_home() } else { PathBuf::from(path) }; let directory = fs::canonicalize(&requested).map_err(|error| format!("Cannot open local folder: {error}"))?;
    let mut entries = fs::read_dir(&directory).map_err(|error| error.to_string())?.filter_map(Result::ok).filter_map(|item| local_entry(item.path()).ok()).collect::<Vec<_>>(); entries.sort_by_key(|entry| (entry.kind != "folder", entry.name.to_lowercase())); Ok(DirectoryListing { path: directory.to_string_lossy().to_string(), entries })
}

fn connect(config: &ConnectionConfig, password: &str) -> Result<Session, String> {
    let stream = TcpStream::connect((config.host.as_str(), config.port)).map_err(|error| format!("Cannot reach server: {error}"))?; let mut session = Session::new().map_err(|error| error.to_string())?; session.set_tcp_stream(stream); session.handshake().map_err(|error| format!("SSH handshake failed: {error}"))?; session.userauth_password(&config.username, password).map_err(|error| format!("Authentication failed: {error}"))?; if !session.authenticated() { return Err("The server did not accept these credentials".into()); } Ok(session)
}

fn remote_entry(path: PathBuf, stat: &ssh2::FileStat) -> Option<FileEntry> { let name = path.file_name()?.to_str()?.to_string(); if name == "." || name == ".." { return None; } let is_dir = stat.perm.map(|value| value & 0o170000 == 0o040000).unwrap_or(false); Some(FileEntry { name, path: path.to_string_lossy().to_string(), kind: if is_dir { "folder" } else { "file" }.into(), size: stat.size.unwrap_or(0), modified: stat.mtime.unwrap_or(0) as u64 * 1000, extension: extension(&path), permissions: stat.perm.map(|value| format!("{value:o}")) }) }
fn list_remote(sftp: &Sftp, path: &Path) -> Result<Vec<FileEntry>, String> { let mut entries = sftp.readdir(path).map_err(|error| format!("Cannot list remote folder: {error}"))?.into_iter().filter_map(|(entry_path, stat)| remote_entry(entry_path, &stat)).collect::<Vec<_>>(); entries.sort_by_key(|entry| (entry.kind != "folder", entry.name.to_lowercase())); Ok(entries) }

#[tauri::command]
fn list_remote_directory(config: ConnectionConfig, password: String, path: String) -> Result<DirectoryListing, String> { let session = connect(&config, &password)?; let sftp = session.sftp().map_err(|error| error.to_string())?; Ok(DirectoryListing { path: path.clone(), entries: list_remote(&sftp, Path::new(&path))? }) }

fn matches(entry: &FileEntry, criteria: &SearchCriteria) -> bool { let query = criteria.query.to_lowercase(); if !query.is_empty() && !entry.name.to_lowercase().contains(&query) { return false; } if criteria.kind != "all" && criteria.kind != entry.kind && !(criteria.kind == "pdf" && entry.extension == "pdf") && !(criteria.kind == "image" && ["png", "jpg", "jpeg", "gif", "webp"].contains(&entry.extension.as_str())) { return false; } let days = match criteria.date.as_str() { "today" => 1, "week" => 7, "month" => 30, _ => 0 }; days == 0 || entry.modified >= (SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64).saturating_sub(days * 86_400_000) }
fn content_matches(path: &Path, criteria: &SearchCriteria) -> bool { if !criteria.content || criteria.query.is_empty() { return true; } let Ok(metadata) = fs::metadata(path) else { return false; }; if metadata.len() > 2_000_000 { return false; } let Ok(contents) = fs::read_to_string(path) else { return false; }; contents.to_lowercase().contains(&criteria.query.to_lowercase()) }
fn search_local_path(root: &Path, criteria: &SearchCriteria, results: &mut Vec<FileEntry>) { let Ok(items) = fs::read_dir(root) else { return; }; for item in items.flatten() { let path = item.path(); let Ok(entry) = local_entry(path.clone()) else { continue; }; if !entry.name.starts_with('.') && matches(&entry, criteria) && content_matches(&path, criteria) { results.push(entry.clone()); } if entry.kind == "folder" { search_local_path(&path, criteria, results); } } }

#[tauri::command]
fn search_local(root: String, criteria: SearchCriteria) -> Result<Vec<serde_json::Value>, String> { let root_path = if root.is_empty() { dirs_home() } else { PathBuf::from(root) }; let mut entries = Vec::new(); search_local_path(&root_path, &criteria, &mut entries); Ok(entries.into_iter().map(|entry| { let path = entry.path.clone(); serde_json::json!({ "side": "local", "relative_path": path, "name": entry.name, "path": entry.path, "kind": entry.kind, "size": entry.size, "modified": entry.modified, "extension": entry.extension, "permissions": entry.permissions }) }).collect()) }

fn search_remote_path(sftp: &Sftp, root: &Path, criteria: &SearchCriteria, results: &mut Vec<FileEntry>) { let Ok(entries) = list_remote(sftp, root) else { return; }; for entry in entries { let path = PathBuf::from(&entry.path); if !entry.name.starts_with('.') && matches(&entry, criteria) { results.push(entry.clone()); } if entry.kind == "folder" { search_remote_path(sftp, &path, criteria, results); } } }
#[tauri::command]
fn search_remote(config: ConnectionConfig, password: String, root: String, criteria: SearchCriteria) -> Result<Vec<serde_json::Value>, String> { let session = connect(&config, &password)?; let sftp = session.sftp().map_err(|error| error.to_string())?; let mut entries = Vec::new(); search_remote_path(&sftp, Path::new(&root), &criteria, &mut entries); Ok(entries.into_iter().map(|entry| { let path = entry.path.clone(); serde_json::json!({ "side": "remote", "relative_path": path, "name": entry.name, "path": entry.path, "kind": entry.kind, "size": entry.size, "modified": entry.modified, "extension": entry.extension, "permissions": entry.permissions }) }).collect()) }

#[tauri::command]
fn transfer_file(config: ConnectionConfig, password: String, direction: String, local_path: String, remote_path: String) -> Result<String, String> { let session = connect(&config, &password)?; let sftp = session.sftp().map_err(|error| error.to_string())?; if direction == "upload" { let mut source = fs::File::open(&local_path).map_err(|error| error.to_string())?; let mut target = sftp.create(Path::new(&remote_path)).map_err(|error| format!("Cannot create remote file: {error}"))?; std::io::copy(&mut source, &mut target).map_err(|error| error.to_string())?; Ok(format!("Uploaded {}", Path::new(&local_path).file_name().and_then(|name| name.to_str()).unwrap_or("file"))) } else { let mut source = sftp.open(Path::new(&remote_path)).map_err(|error| format!("Cannot open remote file: {error}"))?; let filename = Path::new(&remote_path).file_name().and_then(|name| name.to_str()).unwrap_or("download"); let destination = dirs_home().join("Downloads").join(filename); let mut target = fs::File::create(&destination).map_err(|error| error.to_string())?; let mut buffer = Vec::new(); source.read_to_end(&mut buffer).map_err(|error| error.to_string())?; target.write_all(&buffer).map_err(|error| error.to_string())?; Ok(format!("Downloaded to {}", destination.to_string_lossy())) } }

#[tauri::command]
fn save_credential(profile_id: String, password: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    { let status = Command::new("security").args(["add-generic-password", "-a", &profile_id, "-s", "com.harbor.file", "-w", &password, "-U"]).status().map_err(|error| error.to_string())?; if !status.success() { return Err("macOS Keychain rejected the credential".into()); } }
    #[cfg(not(target_os = "macos"))]
    { let _ = (profile_id, password); }
    Ok(())
}
#[tauri::command]
fn get_credential(profile_id: String) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    { let output = Command::new("security").args(["find-generic-password", "-a", &profile_id, "-s", "com.harbor.file", "-w"]).output().map_err(|error| error.to_string())?; if !output.status.success() { return Ok(String::new()); } return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string()); }
    #[cfg(not(target_os = "macos"))]
    { let _ = profile_id; Ok(String::new()) }
}

#[tauri::command]
fn reveal_local_path(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    { let status = Command::new("open").args(["-R", &path]).status().map_err(|error| error.to_string())?; if !status.success() { return Err("Could not reveal the file in Finder".into()); } }
    #[cfg(not(target_os = "macos"))]
    { let _ = path; }
    Ok(())
}

fn main() { tauri::Builder::default().invoke_handler(tauri::generate_handler![read_local_directory, list_remote_directory, search_local, search_remote, transfer_file, save_credential, get_credential, reveal_local_path]).run(tauri::generate_context!()).expect("error while running Harbor File"); }

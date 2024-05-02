use std::error::Error;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use zip::{ZipWriter, CompressionMethod, write::FileOptions};
use rusqlite::{Connection, Result};
use uuid::Uuid;
use serde::Deserialize;
use serde_json;
use tokio_cron_scheduler::{Job, JobScheduler, JobSchedulerError};
use tokio;


struct Backup {
    id: i32,
    path: String,
    uuid: String,
    created_at: String
}

#[derive(Deserialize)]
struct Directory {
    name: String,
    cron: String,
    max_backups: i32,
    max_age: u64,
    input: String,
    output: String
}

#[derive(Deserialize)]
struct Config {
    directories: Vec<Directory>
}

#[tokio::main]
#[allow(unreachable_code)] // Intended to run indefinitely
async fn main() -> Result<(), JobSchedulerError> {
    let sched = JobScheduler::new().await?;

    let config = parse_config();

    // Create a job for every entry in the config
    for directory in config.directories {
        sched.add(
            Job::new(directory.cron.as_str(), move |_uuid, _l| {
                zip_directory(directory.input.clone(), directory.output.clone()).unwrap();
                delete_old_zips(directory.max_age, directory.max_backups);
                println!("Backup of {} completed", directory.name);
            })?
        ).await?;
    }

    sched.start().await?;

    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
    }

    Ok(())
}

fn parse_config() -> Config {
    let mut config_file = File::open("config.json").unwrap();
    let mut buffer = String::new();
    config_file.read_to_string(&mut buffer).unwrap();
    let config_text = buffer.clone();
    let config: Config = serde_json::from_str(&config_text.as_str()).unwrap();

    config
}

fn delete_old_zips(max_age: u64, max_backups: i32) {
    // Delete zip files older than 30 days
    let connection = Connection::open("db.sqlite").unwrap();
    let mut statement = connection.prepare(
        &format!("SELECT * FROM backups WHERE created_at < date('now', '-{} milliseconds')", max_age),
    ).unwrap();
    let backups_result = statement.query_map([], |row| {
        Ok(Backup {
            id: row.get(0).unwrap(),
            path: row.get(1).unwrap(),
            uuid: row.get(2).unwrap(),
            created_at: row.get(3).unwrap()
        })
    }).unwrap();

    let backups: Result<Vec<_>, _> = backups_result.collect();
    let backups = backups.unwrap();

    statement.finalize().unwrap();

    connection.execute(
        "DELETE FROM backups WHERE created_at < date('now', '-30 days')",
        ()
    ).unwrap();

    connection.close().unwrap();

    for backup in backups {
        let zip_name = format!("{}/{}-{}.zip", backup.path, backup.created_at, backup.uuid);
        let zip_file_path = Path::new(&zip_name);
        std::fs::remove_file(zip_file_path).unwrap();
    }
    
}

fn zip_directory(input: String, output: String) -> Result<(), Box<dyn Error>> {
    // Create a new file format: date-uuid.zip
    let date = chrono::Local::now();
    // take input path and convert to sha256 uuid
    let input_path = Path::new(&input).canonicalize()?;

    let input_uuid = Uuid::new_v4();

    // Stores uuid in db and relate to absolute path
    let connection = Connection::open("db.sqlite").unwrap();
    
    connection.execute(
        "INSERT INTO backups (path, uuid, created_at) VALUES (?, ?, ?)",
        &[input_path.to_str().unwrap(), &input_uuid.to_string(), date.to_string().as_str()]
    ).unwrap();

    connection.close().unwrap();
    let zip_name = format!("{}/{}-{}.zip", output, date.timestamp(), input_uuid);
    println!("Creating zip file: {}", zip_name);
    let zip_file_path = Path::new(&zip_name);
    let zip_file = File::create(&zip_file_path).unwrap();

    let mut zip = ZipWriter::new(zip_file);
    let files_to_zip = walk_dir(input.clone());

    println!("Files to zip: {:?}", files_to_zip);

    let options = FileOptions::default()
        .compression_method(CompressionMethod::DEFLATE);

    for path in &files_to_zip {
        let file = File::open(path).unwrap();
        let file_path = path.strip_prefix(&input.clone()).unwrap().to_str().unwrap().to_string();

        println!("Adding file: {}", file_path);

        zip.start_file(file_path, options).unwrap();

        let mut buffer = Vec::new();
        io::copy(&mut file.take(u64::MAX), &mut buffer).unwrap();

        zip.write_all(&buffer).unwrap();
    }

    zip.finish().unwrap();

    println!("Zip file created at {:?}", zip_file_path);

    Ok(())
}

// Recursively walk through directory and return all files
fn walk_dir(input: String) -> Vec<PathBuf> {
    let mut files_to_zip = Vec::new();
    let path = Path::new(&input);
    let entries = path.read_dir().unwrap();

    for entry in entries {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            let sub_files = walk_dir(entry.path().to_str().unwrap().to_string());
            files_to_zip.extend(sub_files);
            continue;
        }
        let path = entry.path();
        files_to_zip.push(path);
    }

    files_to_zip
}
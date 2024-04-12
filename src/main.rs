use std::error::Error;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use zip::{ZipWriter, CompressionMethod, write::FileOptions};
use rusqlite::{Connection, Result};
use uuid::Uuid;
extern crate cronjob;
use cronjob::CronJob;

struct Backup {
    id: i32,
    path: String,
    uuid: String,
    created_at: String
}

struct Directory {
    input: String,
    output: String
}

fn main() {
    let connection = Connection::open("db.sqlite").unwrap();
    connection.execute(
        "CREATE TABLE IF NOT EXISTS backups (
            id INTEGER PRIMARY KEY,
            path TEXT NOT NULL,
            uuid TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
        ()
    ).unwrap();

    connection.execute(
        "CREATE TABLE IF NOT EXISTS directories (
            id INTEGER PRIMARY KEY,
            input TEXT NOT NULL,
            output TEXT NOT NULL
        )",
        ()
    ).unwrap();
    connection.close().unwrap();

    // add_new_directory("./content", "./backups");

    // let mut clean = CronJob::new("Delete Old Files", delete_old_zips);
    // // Run once an hour
    // clean.minutes("0");
    // clean.hours("*");

    // CronJob::start_job_threaded(clean);

    // let mut backup_cronjob = CronJob::new("Backup Directories", backup);
    // // Run once a day
    // backup_cronjob.minutes("0");
    // backup_cronjob.hours("0");
    // backup_cronjob.day_of_month("*");

    // CronJob::start_job_threaded(backup_cronjob);
    delete_old_zips();
    backup();
}

fn backup() {
    let connection = Connection::open("db.sqlite").unwrap();
    let mut statement = connection.prepare("SELECT * FROM directories").unwrap();
    let directories_result = statement.query_map([], |row| {
        Ok(Directory {
            input: row.get(1).unwrap(),
            output: row.get(2).unwrap()
        })
    }).unwrap();

    let directories: Result<Vec<_>, _> = directories_result.collect();
    let directories = directories.unwrap();

    statement.finalize().unwrap();
    connection.close().unwrap();

    for directory in directories {
        zip_directory(directory.input, directory.output).unwrap();
    }
}

fn add_new_directory(input: &str, output: &str) {
    let input_path = Path::new(input);
    if !input_path.exists() {
        panic!("Path does not exist");
    }
    let input_absolute_path = input_path.canonicalize().unwrap();
    let input_absolute_string = input_absolute_path.to_string_lossy();

    let output_path = Path::new(output);
    let output_absolute_path = output_path.canonicalize().unwrap();
    let output_absolute_string = output_absolute_path.to_string_lossy();


    let connection = Connection::open("db.sqlite").unwrap();
    connection.execute(
        "INSERT INTO directories (input, output) VALUES (?, ?)",
        &[&input_absolute_string, &output_absolute_string]
    ).unwrap();
    connection.close().unwrap();
}

fn delete_old_zips() {
    // Delete zip files older than 30 days
    let connection = Connection::open("db.sqlite").unwrap();
    let mut statement = connection.prepare(
        "SELECT * FROM backups WHERE created_at < date('now', '-30 days')",
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
        "DELETE FROM backups WHERE created_at < date('now', '30 days')",
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

// Recursivley walk through directory and return all files
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
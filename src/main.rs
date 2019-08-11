#[macro_use]
extern crate cfg_if;
extern crate time;

use std::fs::{read_dir, Metadata};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;

mod command;
mod result_code;

use command::*;
use result_code::*;

cfg_if! {
    if #[cfg(windows)] {
        fn get_file_info(meta: &Metadata) -> (time::Tm, u64) {
            use std::os::windows::prelude::*;
            (time::at(time::Timespec::new(meta.last_write_time())), meta.file_size())
        }
    } else {
        fn get_file_info(meta: &Metadata) -> (time::Tm, u64) {
            use std::os::unix::prelude::*;
            (time::at(time::Timespec::new(meta.mtime(), 0)), meta.size())
        }
    }
}

fn main() {
    let listener = TcpListener::bind("0.0.0.0:1234").expect("Couldn't bind this address");

    println!("Waiting for clients to connect...");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                thread::spawn(move || {
                    handle_client(stream);
                });
            }
            _ => println!("A client tried to connect..."),
        }
    }
}

#[allow(dead_code)]
struct Client {
    cwd: PathBuf,
    stream: TcpStream,
    name: Option<String>,
    data_writer: Option<TcpStream>,
}

impl Client {
    fn new(stream: TcpStream) -> Self {
        Client {
            cwd: PathBuf::from("/"),
            stream: stream,
            name: None,
            data_writer: None,
        }
    }
    fn handle_cmd(&mut self, cmd: Command) {
        println!("====> {:?}", cmd);
        match cmd {
            Command::Auth => send_cmd(
                &mut self.stream,
                ResultCode::CommandNotImplemented,
                "Not Implemented",
            ),
            Command::Cwd(pb) => send_cmd(&mut self.stream, ResultCode::Ok, "Hahahahaha"),
            Command::List => {
                if let Some(ref mut data_writer) = self.data_writer {
                    let tmp = PathBuf::from(".");
                    send_cmd(
                        &mut self.stream,
                        ResultCode::DataConnectionAlreadyOpen,
                        "Starting to list directory...",
                    );
                    let mut out = String::new();
                    for dir in read_dir(tmp).unwrap() {
                        for entry in dir {
                            add_file_info(entry.path(), &mut out);
                        }
                        send_data(data_writer, &out)
                    }
                } else {
                    send_cmd(
                        &mut self.stream,
                        ResultCode::ConnectionClosed,
                        "No opened data connection",
                    );
                }
                if self.data_writer.is_some() {
                    self.data_writer = None;
                    send_cmd(
                        &mut self.stream,
                        ResultCode::ClosingDataConnection,
                        "Transfer done",
                    );
                }
            }
            Command::NoOp => send_cmd(&mut self.stream, ResultCode::Ok, "Doing nothing..."),
            Command::Pwd => {
                let msg = format!("{}", self.cwd.to_str().unwrap_or(""));
                if !msg.is_empty() {
                    let message = format!("\"{}\" ", msg);
                    send_cmd(&mut self.stream, ResultCode::PATHNAMECreated, &message);
                } else {
                    send_cmd(
                        &mut self.stream,
                        ResultCode::FileNotFound,
                        "No such file or directory",
                    );
                }
            }
            Command::Pasv => {
                if self.data_writer.is_some() {
                    send_cmd(
                        &mut self.stream,
                        ResultCode::DataConnectionAlreadyOpen,
                        "Already listening...",
                    )
                } else {
                    let port: u16 = 43210;
                    send_cmd(
                        &mut self.stream,
                        ResultCode::EnteringPassiveMode,
                        &format!("127,0,0,1,{},{}", port >> 8, port & 0xFF),
                    );
                    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port);
                    let listener = TcpListener::bind(&addr).unwrap();
                    match listener.incoming().next() {
                        Some(Ok(client)) => {
                            self.data_writer = Some(client);
                        }
                        _ => {
                            send_cmd(
                                &mut self.stream,
                                ResultCode::ServiceNotAvailable,
                                "bad things happen to good servers...",
                            );
                        }
                    }
                }
            }
            Command::Syst => send_cmd(&mut self.stream, ResultCode::Ok, "No"),
            Command::Type => send_cmd(
                &mut self.stream,
                ResultCode::Ok,
                "Transfer type changed successfully",
            ),
            Command::User(username) => {
                if username.is_empty() {
                    send_cmd(
                        &mut self.stream,
                        ResultCode::InvalidParameterOrArgument,
                        "Invalid username",
                    );
                } else {
                    self.name = Some(username.to_owned());
                    send_cmd(
                        &mut self.stream,
                        ResultCode::UserLoggedIn,
                        &format!("Welcome {}!", username),
                    );
                }
            }
            Command::Unknown(s) => send_cmd(
                &mut self.stream,
                ResultCode::UnknownCommand,
                &format!("Unknown command {}", s),
            ),
        }
    }
}

fn handle_client(mut stream: TcpStream) {
    println!("new client connected!");
    send_cmd(
        &mut stream,
        ResultCode::ServiceReadyForNewUser,
        "Welcome to this FTP server!",
    );
    let mut client = Client::new(stream);
    loop {
        let data = read_all_message(&mut client.stream);
        if data.is_empty() {
            println!("client disconnected...");
            break;
        }
        client.handle_cmd(Command::new(data).unwrap());
    }
}

fn send_cmd(stream: &mut TcpStream, code: ResultCode, message: &str) {
    let msg = if message.is_empty() {
        format!("{}\r\n", code as u32)
    } else {
        format!("{} {}\r\n", code as u32, message)
    };
    println!("<==== {}", msg);
    write!(stream, "{}", msg).unwrap()
}

fn read_all_message(stream: &mut TcpStream) -> Vec<u8> {
    let buf = &mut [0; 1];
    let mut out = Vec::with_capacity(100);

    loop {
        match stream.read(buf) {
            Ok(received) if received > 0 => {
                if out.is_empty() && buf[0] == b' ' {
                    continue;
                }
                out.push(buf[0]);
            }
            _ => return Vec::new(),
        }
        let len = out.len();
        if len > 1 && out[len - 2] == b'\r' && out[len - 1] == b'\n' {
            out.pop();
            out.pop();
            return out;
        }
    }
}

fn send_data(stream: &mut TcpStream, s: &str) {
    write!(stream, "{}", s).unwrap();
}

fn add_file_info(path: PathBuf, out: &mut String) {
    let extra = if path.is_dir() { "/" } else { "" };
    let is_dir = if path.is_dir() { "d" } else { "-" };
    let meta = match ::std::fs::metadata(&path) {
        Ok(meta) => meta,
        _ => return,
    };
    let (time, file_size) = get_file_info(&meta);
    let path = match path.to_str() {
        Some(path) => match path.split("/").last() {
            Some(path) => path,
            _ => return,
        },
        _ => return,
    };
    let rights = if meta.permissions().readonly() {
        "r--r--r--"
    } else {
        "rw-rw-rw-"
    };
    let file_str = format!("{is_dir} {rights}  {links}  {owner} {group}  {size}  {month} {day} {hour}:{min}  {path}{extra}\r\n", is_dir = is_dir, rights = rights, links = 1, owner = "anonymous", group = "anonymous", size = file_size, month = time.tm_mon, day = time.tm_mday, hour = time.tm_hour, min = time.tm_min, path = path, extra = extra);
    out.push_str(&file_str);
    println!("====> {:?}", &file_str);
}

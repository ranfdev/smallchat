use mio::net::{TcpListener, TcpStream};
use mio::{Events, Interest, Poll, Token};
use std::collections::BTreeMap;
use std::io::{self, prelude::*};
use std::rc::Rc;

const SERVER: Token = Token(0);
const BUFLEN: usize = 4096;

struct OutboxItem {
    // Using an Rc lets me share a single Buffer with multiple clients.
    // Even if there are 1000 clients, for a broadcast message there will be only one
    // shared `Vec<u8>` in memory
    data: Rc<Vec<u8>>,
    cursor: usize,
}
struct Client {
    nick: String,
    listener: TcpStream,
    read_buf: Box<[u8; BUFLEN]>,
    read_buf_start: usize,
    outbox: Vec<OutboxItem>,
    writable: bool,
}

impl Client {
    fn write(&mut self, data: impl Into<Rc<Vec<u8>>>) -> Result<(), io::Error> {
        self.outbox.push(OutboxItem {
            data: data.into(),
            cursor: 0,
        });
        if self.writable {
            self.flush_outbox()?;
        }
        Ok(())
    }
    fn flush_outbox(&mut self) -> Result<(), io::Error> {
        while !self.outbox.is_empty() {
            let cursor = self.outbox[0].cursor;
            match self.listener.write(&self.outbox[0].data[cursor..]) {
                Ok(0) => {
                    self.outbox.remove(0);
                    break;
                }
                Ok(n) => {
                    self.outbox[0].cursor += n;
                }
                Err(e) if is_would_block(&e) => {
                    break;
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
}

struct Chat {
    clients: BTreeMap<Token, Client>,
    max_client: Token,
}

impl Chat {
    fn new() -> Self {
        Self {
            clients: Default::default(),
            max_client: Token(0),
        }
    }
    fn push_from(&mut self, src: &Token, data: Vec<u8>) {
        let data = Rc::new(data);
        for (_, c) in self.clients.iter_mut().filter(|(k, _)| *k != src) {
            c.write(data.clone()).unwrap();
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut chat = Chat::new();
    let mut poll = Poll::new()?;
    let addr = "127.0.0.1:7711".parse().unwrap();

    let mut server = TcpListener::bind(addr)?;
    println!("Server started at {addr}");
    poll.registry()
        .register(&mut server, SERVER, Interest::READABLE)?;

    let mut events = Events::with_capacity(1024);

    loop {
        poll.poll(&mut events, None)?;
        for event in &events {
            let token = event.token();
            if token == Token(0) {
                loop {
                    let (mut conn, addr) = match server.accept() {
                        Ok((conn, addr)) => (conn, addr),
                        Err(e) if is_would_block(&e) => break,
                        Err(e) => {
                            return Err(e.into());
                        }
                    };
                    let next_client = Token(chat.max_client.0 + 1);
                    poll.registry().register(
                        &mut conn,
                        next_client,
                        Interest::READABLE | Interest::WRITABLE,
                    )?;
                    let mut client = Client {
                        nick: format!("user:{}", next_client.0),
                        listener: conn,
                        read_buf: Box::new([0; 4096]),
                        read_buf_start: 0,
                        outbox: Default::default(),
                        writable: false,
                    };
                    client.write(
                        "Welcome to Simple Chat!\nUse /nick <nick> to set your nick.\n> "
                            .to_string()
                            .into_bytes(),
                    )?;
                    chat.clients.insert(next_client, client);
                    chat.max_client = next_client;
                    println!("Connected client from {addr}");
                }
            } else {
                if event.is_readable() {
                    let mut finished = false;
                    loop {
                        let client = chat.clients.get_mut(&token).unwrap();
                        match client
                            .listener
                            .read(&mut client.read_buf[client.read_buf_start..])
                        {
                            Ok(0) => {
                                finished = true;
                                break;
                            }
                            Ok(n) => {
                                client.read_buf_start += n;
                            }
                            Err(e) if is_would_block(&e) => {
                                break;
                            }
                            Err(e) => {
                                return Err(e.into());
                            }
                        }
                    }
                    let mut start = 0;
                    loop {
                        let client = chat.clients.get_mut(&token).unwrap();
                        let Some(len) = client.read_buf[start..client.read_buf_start]
                            .iter()
                            .enumerate()
                            .find(|(_, x)| **x == '\n' as u8)
                            .map(|(i, _)| i)
                        else {
                            break;
                        };

                        let msg = &client.read_buf[start..start + len];

                        // Build response to send
                        let mut res = Vec::new();

                        if let Some(nick) = msg.strip_prefix("/nick ".as_bytes()) {
                            client.nick.clear();
                            if let Ok(nick) = core::str::from_utf8(nick) {
                                client.nick.push_str(nick);
                                res.extend_from_slice("nick changed to ".as_bytes());
                                res.extend_from_slice(nick.as_bytes());
                                res.extend_from_slice("\n> ".as_bytes());

                                client.write(res)?;
                            } else {
                                client.write("invalid nick".as_bytes().to_vec())?;
                            }
                        } else {
                            res.extend_from_slice(client.nick.as_bytes());
                            res.extend_from_slice("> ".as_bytes());
                            res.extend(msg);
                            res.extend_from_slice("\n> ".as_bytes());
                            chat.push_from(&token, res);
                        }
                        start += len + 1;
                    }
                    if finished {
                        let client = chat.clients.get_mut(&token).unwrap();
                        client.read_buf_start = 0;
                    }
                }
                if event.is_writable() {
                    let client = chat.clients.get_mut(&token).unwrap();
                    client.writable = true;
                    client.flush_outbox()?;
                }
            }
        }
    }
}

fn is_would_block(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::WouldBlock
}

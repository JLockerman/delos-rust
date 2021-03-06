
extern crate mio;
extern crate fuzzy_log;

use fuzzy_log::packets::*;
use fuzzy_log::buffer::Buffer;

use mio::net::TcpStream;
//use mio::Poll;

use std::io::{self, Write, Read};
use std::time::{Duration, Instant};
use std::thread;

fn main() {
    let mut args = ::std::env::args().skip(1);
    let mut send_server0 = TcpStream::connect(&args.next().unwrap().parse().unwrap()).unwrap();
    let mut recv_server0 = TcpStream::connect(&args.next().unwrap().parse().unwrap()).unwrap();
    let mut send_server1 = TcpStream::connect(&args.next().unwrap().parse().unwrap()).unwrap();
    let mut recv_server1 = TcpStream::connect(&args.next().unwrap().parse().unwrap()).unwrap();

    send_server0.set_nodelay(true).unwrap();
    recv_server0.set_nodelay(true).unwrap();
    send_server1.set_nodelay(true).unwrap();
    recv_server1.set_nodelay(true).unwrap();
    // let poll = Poll::new().unwrap();
    // poll.register(
    //     &to_server0,
    //     mio::Token(1),
    //     mio::Ready::readable() | mio::Ready::writable(),
    //     mio::PollOpt::edge(),
    // ).unwrap();
    // poll.register(
    //     &to_server1,
    //     mio::Token(1),
    //     mio::Ready::readable() | mio::Ready::writable(),
    //     mio::PollOpt::edge()
    // ).unwrap();

    let client_id = Uuid::new_v4();
    // println!("client {:?}\n", client_id);
    // println!("write  {:?}", write_id);

    blocking_write(&mut send_server0, &*client_id.as_bytes()).unwrap();
    blocking_write(&mut recv_server0, &*client_id.as_bytes()).unwrap();
    blocking_write(&mut send_server1, &*client_id.as_bytes()).unwrap();
    blocking_write(&mut recv_server1, &*client_id.as_bytes()).unwrap();

    let mut run_exp = || {
        let write_id = Uuid::new_v4();

        let mut skeens = vec![];
        EntryContents::Multi{
            id: &write_id,
            flags: &(EntryFlag::NewMultiPut | EntryFlag::TakeLock),
            lock: &0,
            locs: &[OrderIndex(2.into(), 0.into()), OrderIndex(3.into(), 0.into())],
            deps: &[],
            data: &[94, 49, 0xff],
        }.fill_vec(&mut skeens);
        skeens.extend_from_slice(&*client_id.as_bytes());

        let mut fence0_and_update = vec![];
        EntryContents::FenceClient{
            fencing_write: &write_id,
            client_to_fence: &client_id,
            fencing_client: &client_id,
        }.fill_vec(&mut fence0_and_update);
        fence0_and_update.extend_from_slice(&*client_id.as_bytes());
        EntryContents::UpdateRecovery {
            old_recoverer: &Uuid::nil(),
            write_id: &write_id,
            flags: &EntryFlag::Nothing,
            lock: &0,
            locs: &[OrderIndex(2.into(), 0.into()), OrderIndex(3.into(), 0.into())],
        }.fill_vec(&mut fence0_and_update);
        fence0_and_update.extend_from_slice(&*client_id.as_bytes());

        let mut fence1 = vec![];
        EntryContents::FenceClient{
            fencing_write: &write_id,
            client_to_fence: &client_id,
            fencing_client: &client_id,
        }.fill_vec(&mut fence1);
        fence1.extend_from_slice(&*client_id.as_bytes());

        let mut check_skeens1 = vec![];
        EntryContents::CheckSkeens1 {
            id: &write_id,
            flags: &EntryFlag::Nothing,
            data_bytes: &0,
            dependency_bytes: &0,
            loc: &OrderIndex(0.into(), 0.into()),
        }.fill_vec(&mut check_skeens1);
        check_skeens1.extend_from_slice(&*client_id.as_bytes());

        let mut buffer = Buffer::new();
        let mut buffer1 = Buffer::new();
        let mut buffer2 = Buffer::new();

        blocking_write(&mut send_server0, &skeens).unwrap();
        recv_packet(&mut buffer, &recv_server0);

        // println!("a");

        let start = Instant::now();

        let reason = buffer.contents().locs()[0];
        let ts0: u32 = reason.1.into();
        assert!(ts0 > 0);

        bytes_as_entry_mut(&mut check_skeens1).locs_mut()[0] = reason;

        blocking_writes(&mut [
            (&mut send_server0, &fence0_and_update, false),
            (&mut send_server1, &fence1, false)
        ]);

        // println!("b");

        recv_packets(&mut [
            (&mut buffer, &send_server0, 0, false),
            (&mut buffer1, &send_server1, 0, false),
            (&mut buffer2, &recv_server0, 0, false),
        ]);
        match buffer.contents() {
            EntryContents::FenceClient{fencing_write, client_to_fence, fencing_client,} => {
                assert_eq!(fencing_write, &write_id);
                assert_eq!(client_to_fence, &client_id);
                assert_eq!(fencing_client, &client_id);
            }
            c => panic!("{:?}", c),
        }

        // println!("c");
        match buffer1.contents() {
            EntryContents::FenceClient{fencing_write, client_to_fence, fencing_client,} => {
                assert_eq!(fencing_write, &write_id);
                assert_eq!(client_to_fence, &client_id);
                assert_eq!(fencing_client, &client_id);
            }
            _ => unreachable!(),
        }

        // println!("d");
        match buffer2.contents() {
            EntryContents::UpdateRecovery{flags, ..} =>
                assert!(flags.contains(EntryFlag::ReadSuccess)),
            _ => unreachable!(),
        }

        // println!("e");

        blocking_write(&mut send_server0, &check_skeens1).unwrap();
        recv_packet(&mut buffer, &send_server0);
        match buffer.contents() {
            EntryContents::CheckSkeens1{id, flags, loc, ..} => {
                assert_eq!(id, &write_id);
                assert_eq!(loc, &reason);
                assert!(flags.contains(EntryFlag::ReadSuccess));
            }
            _ => unreachable!(),
        }

        // println!("f");
        //skeens1 is idempotent so we can use it as a TAS
        blocking_write(&mut send_server1, &skeens).unwrap();
        recv_packet(&mut buffer, &recv_server1);
        let ts1: u32 = buffer.contents().locs()[1].1.into();
        assert!(ts1 > 0);

        // println!("g");

        {
            let mut e = bytes_as_entry_mut(&mut skeens);
            *e.lock_mut() = ::std::cmp::max(ts0 as u64, ts1 as u64);
            e.flag_mut().insert(EntryFlag::Unlock);
        }
        blocking_writes(&mut [
            (&mut send_server0, &skeens, false),
            (&mut send_server1, &skeens, false)
        ]);

        // println!("h");

        recv_packets(&mut [
            (&mut buffer, &recv_server0, 0, false),
            (&mut buffer1, &recv_server1, 0, false),
        ]);


        // println!("i");

        let elapsed = start.elapsed();
        println!("{:?}", elapsed);
        elapsed
    };

    let mut data_points = Vec::with_capacity(1000);
    for _ in 0..data_points.capacity() {
        let point = run_exp();
        data_points.push(point);
    }

    let sum: Duration = data_points.iter().sum();
    let avg: Duration = sum / (data_points.len() as u32);
    println!("avg of {:?} runs\n\t {:?}", data_points.len(), avg);
}

fn blocking_write<W: Write>(w: &mut W, mut buffer: &[u8]) -> io::Result<()> {
    //like Write::write_all but doesn't die on WouldBlock
    'recv: while !buffer.is_empty() {
        match w.write(buffer) {
            Ok(i) => { let tmp = buffer; buffer = &tmp[i..]; }
            Err(e) => match e.kind() {
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted => {
                    thread::yield_now();
                    continue 'recv
                },
                _ => { return Err(e) }
            }
        }
    }
    if !buffer.is_empty() {
        return Err(io::Error::new(io::ErrorKind::WriteZero,
            "failed to fill whole buffer"))
    }
    else {
        return Ok(())
    }
}

fn blocking_writes(
    state: &mut [(&mut TcpStream, &[u8], bool)],
) {
    state.iter_mut().fold((), |_, s| { s.2 = false });
    //like Write::write_all but doesn't die on WouldBlock
    let mut done = 0;
    while done < state.len() {
        'poll: for &mut (ref mut w, ref mut buffer, ref mut finished) in state.iter_mut() {
            'recv: while !buffer.is_empty() {
                if *finished { continue 'poll }
                match w.write(buffer) {
                    Ok(i) => { let tmp = *buffer; *buffer = &tmp[i..]; }
                    Err(e) => match e.kind() {
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted => {
                            thread::yield_now();
                            continue 'poll
                        },
                        e => panic!("{:?}", e),
                    }
                }
            }
            *finished = true;
            done += 1;
            continue 'poll
        }
    }
}

fn recv_packet(buffer: &mut Buffer, mut stream: &TcpStream) {
    use fuzzy_log::packets::Packet::WrapErr;
    let mut read = 0;
    loop {
        let to_read = buffer.finished_at(read);
        let size = match to_read {
            Err(WrapErr::NotEnoughBytes(needs)) => needs,
            Err(err) => panic!("{:?}", err),
            Ok(size) if read < size => size,
            Ok(..) => return,
        };
        let r = stream.read(&mut buffer[read..size]);
        match r {
            Ok(i) => read += i,

            Err(e) => match e.kind() {
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted => {
                    thread::yield_now();
                    continue
                },
                _ => panic!("recv error {:?}", e),
            }
        }
    }
}

/*fn recv_2packet(
    buffer0: &mut Buffer, buffer1: &mut Buffer,
    mut stream0: &TcpStream, mut stream1: &TcpStream
) {
    use fuzzy_log::packets::Packet::WrapErr;
    let mut read0 = 0;
    let mut read1 = 0;
    let mut recving0 = true;
    let mut recving1 = true;
    while recving0 || recving1 {
        if recving0 {
            let to_read = buffer0.finished_at(read0);
            let size = match to_read {
                Err(WrapErr::NotEnoughBytes(needs)) => needs,
                Err(err) => panic!("{:?}", err),
                Ok(size) if read < size => size,
                Ok(..) => recving0 = false,
            };
            let r = stream0.read(&mut buffer0[read..size]);
            match r {
                Ok(i) => read0 += i,

                Err(e) => match e.kind() {
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted => {},
                    _ => panic!("recv error {:?}", e),
                }
            }
        }
        if recving1 {
            let to_read = buffer.finished_at(read1);
            let size = match to_read {
                Err(WrapErr::NotEnoughBytes(needs)) => needs,
                Err(err) => panic!("{:?}", err),
                Ok(size) if read < size => size,
                Ok(..) => recving1 = false,
            };
            let r = stream.read(&mut buffer1[read..size]);
            match r {
                Ok(i) => read += i,

                Err(e) => match e.kind() {
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted => {},
                    _ => panic!("recv error {:?}", e),
                }
            }
        }
    }
}*/

fn recv_packets(
    state: &mut [(&mut Buffer, &TcpStream, usize, bool)],
) {
    use fuzzy_log::packets::Packet::WrapErr;
    state.iter_mut().fold((), |_, s| { s.2 = 0; s.3 = false; });
    let mut done = 0;
    while done < state.len() {
        'poll: for &mut (ref mut buffer, ref mut stream, ref mut read, ref mut finished) in
            state.iter_mut() {
            if *finished {
                continue 'poll
            }
            let to_read = buffer.finished_at(*read);
            let size = match to_read {
                Err(WrapErr::NotEnoughBytes(needs)) => needs,
                Err(err) => panic!("{:?}", err),
                Ok(size) if *read < size => size,
                Ok(..) => {
                    done += 1;
                    *finished = true;
                    continue 'poll
                },
            };
            let r = stream.read(&mut buffer[*read..size]);
            match r {
                Ok(i) => {
                    *read = *read + i
                },

                Err(e) => match e.kind() {
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted => {},
                    _ => panic!("recv error {:?}", e),
                }
            }
        }
    }
}

use std::{io, os::windows::io::AsRawHandle, time::Duration};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::windows::named_pipe::{NamedPipeServer, ServerOptions},
};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, SECURITY_ATTRIBUTES},
    System::Pipes::GetNamedPipeClientProcessId,
};

use crate::identity::Peer;

pub fn pipe_name(nonce: &str) -> io::Result<String> {
    if nonce.len() != 32 || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid rendezvous ID"));
    }
    Ok(format!(r"\\.\pipe\Porthole.ConsoleWorker.{nonce}"))
}

fn server(name: &str, logon: &str) -> io::Result<NamedPipeServer> {
    // No generic-write right: it also grants FILE_CREATE_PIPE_INSTANCE. The
    // Client rights include read/write data, attributes, EA, READ_CONTROL and
    // SYNCHRONIZE, but explicitly exclude the pipe-instance creation bit.
    let sddl: Vec<u16> = format!("D:P(A;;GA;;;SY)(A;;0x0012019b;;;{logon})S:(ML;;NW;;;ME)")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut descriptor = std::ptr::null_mut();
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut descriptor, std::ptr::null_mut()) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let result = ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .max_instances(1)
            .in_buffer_size(64)
            .out_buffer_size(64)
            .create_with_security_attributes_raw(name, (&mut attributes as *mut SECURITY_ATTRIBUTES).cast());
        LocalFree(descriptor);
        result
    }
}

fn authenticate(pipe: &NamedPipeServer, helper: &Peer) -> io::Result<()> {
    let mut pid = 0;
    if unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle(), &mut pid) } == 0 || pid != helper.pid || !helper.alive() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Unexpected pipe client"));
    }
    Ok(())
}

async fn expect<S: tokio::io::AsyncRead + Unpin>(stream: &mut S, expected: &[u8; 4]) -> io::Result<()> {
    let mut message = [0; 4];
    stream.read_exact(&mut message).await?;
    if &message != expected {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Unexpected protocol message"));
    }
    Ok(())
}

#[cfg(test)]
async fn exchange<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    stream: &mut S,
    validate: impl Fn() -> io::Result<()>,
) -> io::Result<()> {
    exchange_operation(stream, validate, false, || async { Ok(*b"DONE") }).await
}

async fn exchange_operation<S, F, T>(stream: &mut S, validate: impl Fn() -> io::Result<()>, transfer: bool, operation: F) -> io::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    F: FnOnce() -> T,
    T: std::future::Future<Output = io::Result<[u8; 4]>>,
{
    validate()?;
    stream.write_all(if transfer { b"RDY2" } else { b"RDY1" }).await?;
    expect(stream, if transfer { b"CMT2" } else { b"CMT1" }).await?;
    validate()?;
    let result = operation().await?;
    stream.write_all(&result).await?;
    expect(stream, b"ACK1").await
}

pub fn run(helper: &Peer, worker: &Peer, expected: &std::path::Path, nonce: &str) -> io::Result<()> {
    let name = pipe_name(nonce)?;
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
        let mut pipe = server(&name, &helper.identity.logon)?;
        tokio::time::timeout(Duration::from_secs(30), async {
            pipe.connect().await?;
            authenticate(&pipe, helper)?;
            exchange_operation(
                &mut pipe,
                || helper.authorize_helper(worker, expected),
                true,
                || crate::handoff::transfer(worker.identity.session),
            )
            .await
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Rendezvous expired"))?
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn handoff_requires_its_own_commit_and_executes_only_once() {
        use std::cell::Cell;
        for frame in [b"CMT1", b"XXXX", b"CMT2"] {
            let calls = Cell::new(0);
            let (mut server, mut client) = tokio::io::duplex(64);
            let (result, _) = tokio::join!(
                exchange_operation(
                    &mut server,
                    || Ok(()),
                    true,
                    || {
                        calls.set(calls.get() + 1);
                        async { Ok(*b"DONE") }
                    }
                ),
                async {
                    expect(&mut client, b"RDY2").await.unwrap();
                    client.write_all(frame).await.unwrap();
                    if frame == b"CMT2" {
                        expect(&mut client, b"DONE").await.unwrap();
                        // Replaying commit instead of acknowledging must not execute again.
                        client.write_all(b"CMT2").await.unwrap();
                    }
                }
            );
            assert!(result.is_err());
            assert_eq!(calls.get(), usize::from(frame == b"CMT2"));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn handoff_revalidation_failure_never_invokes_operation() {
        use std::cell::Cell;
        let validations = Cell::new(0);
        let operations = Cell::new(0);
        let (mut server, mut client) = tokio::io::duplex(64);
        let (result, _) = tokio::join!(
            exchange_operation(
                &mut server,
                || {
                    validations.set(validations.get() + 1);
                    if validations.get() == 1 {
                        Ok(())
                    } else {
                        Err(io::Error::other("Peer died"))
                    }
                },
                true,
                || {
                    operations.set(operations.get() + 1);
                    async { Ok(*b"DONE") }
                }
            ),
            async {
                expect(&mut client, b"RDY2").await.unwrap();
                client.write_all(b"CMT2").await.unwrap();
            }
        );
        assert!(result.is_err());
        assert_eq!(operations.get(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn handoff_disconnect_before_commit_never_invokes_operation() {
        use std::cell::Cell;
        let operations = Cell::new(0);
        let (mut server, mut client) = tokio::io::duplex(64);
        let (result, _) = tokio::join!(
            exchange_operation(
                &mut server,
                || Ok(()),
                true,
                || {
                    operations.set(operations.get() + 1);
                    async { Ok(*b"DONE") }
                }
            ),
            async {
                expect(&mut client, b"RDY2").await.unwrap();
                drop(client);
            }
        );
        assert!(result.is_err());
        assert_eq!(operations.get(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn partial_commit_never_invokes_handoff() {
        use std::cell::Cell;
        let operations = Cell::new(0);
        let (mut server, mut client) = tokio::io::duplex(64);
        let (result, _) = tokio::join!(
            exchange_operation(
                &mut server,
                || Ok(()),
                true,
                || {
                    operations.set(operations.get() + 1);
                    async { Ok(*b"DONE") }
                }
            ),
            async {
                expect(&mut client, b"RDY2").await.unwrap();
                client.write_all(b"CM").await.unwrap();
                drop(client);
            }
        );
        assert!(result.is_err());
        assert_eq!(operations.get(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn disconnect_after_commit_does_not_repeat_handoff() {
        use std::cell::Cell;
        let operations = Cell::new(0);
        let (mut server, mut client) = tokio::io::duplex(64);
        let (result, _) = tokio::join!(
            exchange_operation(
                &mut server,
                || Ok(()),
                true,
                || {
                    operations.set(operations.get() + 1);
                    async { Ok(*b"DONE") }
                }
            ),
            async {
                expect(&mut client, b"RDY2").await.unwrap();
                client.write_all(b"CMT2").await.unwrap();
                drop(client);
            }
        );
        assert!(result.is_err());
        assert_eq!(operations.get(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn native_pipe_checks_kernel_pid_and_excludes_second_server() {
        use windows_sys::Win32::{Foundation::INVALID_HANDLE_VALUE, Storage::FileSystem::CreateFileW};
        let mut peer = Peer::open(std::process::id()).unwrap();
        let name = pipe_name(&format!("{:032x}", std::process::id())).unwrap();
        let mut pipe = server(&name, &peer.identity.logon).unwrap();
        assert!(server(&name, &peer.identity.logon).is_err());
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let raw = unsafe { CreateFileW(wide.as_ptr(), 0x0012019b, 0, std::ptr::null(), 3, 0x40100000, std::ptr::null_mut()) };
        assert_ne!(raw, INVALID_HANDLE_VALUE, "{}", io::Error::last_os_error());
        let mut client = unsafe { tokio::net::windows::named_pipe::NamedPipeClient::from_raw_handle(raw) }.unwrap();
        pipe.connect().await.unwrap();
        authenticate(&pipe, &peer).unwrap();
        peer.pid = 0;
        assert!(authenticate(&pipe, &peer).is_err());
        let (result, _) = tokio::join!(exchange(&mut pipe, || Ok(())), async {
            expect(&mut client, b"RDY1").await.unwrap();
            client.write_all(b"CMT1").await.unwrap();
            expect(&mut client, b"DONE").await.unwrap();
            client.write_all(b"ACK1").await.unwrap();
        });
        result.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn revoked_authorization_rejects_commit_without_done_response() {
        use std::cell::Cell;
        let calls = Cell::new(0);
        let (mut server, mut client) = tokio::io::duplex(64);
        let (result, _) = tokio::join!(
            exchange(&mut server, || {
                calls.set(calls.get() + 1);
                if calls.get() == 1 {
                    Ok(())
                } else {
                    Err(io::Error::other("Peer exited"))
                }
            }),
            async {
                expect(&mut client, b"RDY1").await.unwrap();
                client.write_all(b"CMT1").await.unwrap();
            }
        );
        assert!(result.is_err());
        let mut byte = [0];
        assert!(
            tokio::time::timeout(Duration::from_millis(20), client.read(&mut byte))
                .await
                .is_err()
        );
    }

    #[test]
    fn rejects_path_and_command_arguments_as_rendezvous() {
        for invalid in ["", "../worker", "abc /dest:console", "0000000000000000000000000000000g"] {
            assert!(pipe_name(invalid).is_err());
        }
        assert!(pipe_name("0123456789abcdef0123456789abcdef").is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ordered_exchange_and_duplicate_commit_rejection() {
        for final_message in [b"ACK1", b"CMT1"] {
            let (mut server, mut client) = tokio::io::duplex(64);
            let server_side = exchange(&mut server, || Ok(()));
            let client_side = async {
                expect(&mut client, b"RDY1").await.unwrap();
                client.write_all(b"CMT1").await.unwrap();
                expect(&mut client, b"DONE").await.unwrap();
                client.write_all(final_message).await.unwrap();
            };
            let (result, _) = tokio::join!(server_side, client_side);
            assert_eq!(result.is_ok(), final_message == b"ACK1");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn expiry_and_channel_loss_do_not_commit() {
        let (mut server, _client) = tokio::io::duplex(64);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), exchange(&mut server, || Ok(())))
                .await
                .is_err()
        );
        let (mut server, mut client) = tokio::io::duplex(64);
        let (result, _) = tokio::join!(exchange(&mut server, || Ok(())), async {
            expect(&mut client, b"RDY1").await.unwrap();
            drop(client);
        });
        assert!(result.is_err());
    }
}

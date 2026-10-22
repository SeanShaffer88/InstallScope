//! Development-only owned loopback/file fixtures. Inspector makes no connections.
use anyhow::{ensure, Context, Result};
use setupscope_core::{
    devinspect::{self, Identity, Transport},
    snapshot::Progress,
};
use std::{
    fs,
    net::{TcpListener, UdpSocket},
    path::PathBuf,
    process::{Child, Command},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
struct Holder {
    child: Child,
    release: PathBuf,
}
impl Drop for Holder {
    fn drop(&mut self) {
        let _ = fs::write(&self.release, b"release synthetic holder");
    }
}
fn main() -> Result<()> {
    let arguments: Vec<_> = std::env::args_os().collect();
    if arguments.get(1).is_some_and(|a| a == "--hold") {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let path = PathBuf::from(arguments.get(2).context("holder file")?);
            let ready = PathBuf::from(arguments.get(3).context("ready path")?);
            let release = PathBuf::from(arguments.get(4).context("release path")?);
            let _file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(0)
                .open(path)?;
            fs::write(ready, b"ready")?;
            let end = Instant::now() + Duration::from_secs(60);
            while !release.exists() && Instant::now() < end {
                std::thread::sleep(Duration::from_millis(40));
            }
            return Ok(());
        }
        #[cfg(not(windows))]
        anyhow::bail!("Holder fixture requires Windows");
    }
    let parent = PathBuf::from(
        arguments
            .get(1)
            .context("Pass an absolute scratch parent")?,
    );
    ensure!(parent.is_absolute(), "Scratch parent must be absolute");
    fs::create_dir_all(&parent)?;
    let root = parent.join(format!(
        "ports-locks-probe-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    fs::create_dir(&root)?;
    // Query before binding: IP Helper inspection itself does not need Winsock
    // initialization or an application-created socket/connection.
    let cold = devinspect::inspect_ports(false, &Progress::default())?;
    ensure!(
        cold.sources
            .iter()
            .any(|s| s.name == "TCP IPv4" && s.error.is_none()),
        "Cold IPv4 table query unavailable"
    );
    let tcp = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let tcp_port = tcp.local_addr()?.port();
    let udp = UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let udp_port = udp.local_addr()?.port();
    let tcp6 = TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, 0));
    let udp6 = UdpSocket::bind((std::net::Ipv6Addr::LOCALHOST, 0));
    let report = devinspect::inspect_ports(true, &Progress::default())?;
    let pid = std::process::id();
    let found = |transport, port, ipv6| {
        report.endpoints.iter().any(|e| {
            e.pid == pid
                && e.transport == transport
                && e.port == port
                && e.address.is_ipv6() == ipv6
        })
    };
    ensure!(
        found(Transport::Tcp, tcp_port, false) && found(Transport::Udp, udp_port, false),
        "Owned IPv4 listener/binding missing"
    );
    let mut ipv6_evidence = String::new();
    if let Ok(listener) = &tcp6 {
        ensure!(
            found(Transport::Tcp, listener.local_addr()?.port(), true),
            "Owned TCP IPv6 listener missing"
        );
        ipv6_evidence.push_str("TCP IPv6 passed. ");
    } else {
        ipv6_evidence.push_str("TCP IPv6 fixture unavailable. ");
    }
    if let Ok(socket) = &udp6 {
        ensure!(
            found(Transport::Udp, socket.local_addr()?.port(), true),
            "Owned UDP IPv6 binding missing"
        );
        ipv6_evidence.push_str("UDP IPv6 passed.");
    } else {
        ipv6_evidence.push_str("UDP IPv6 fixture unavailable.");
    }
    let file = root.join("held synthetic file-🦀.dat");
    let original = b"synthetic developer lock evidence";
    fs::write(&file, original)?;
    let ready = root.join("ready");
    let release = root.join("release");
    let mut command = Command::new(std::env::current_exe()?);
    command.arg("--hold").arg(&file).arg(&ready).arg(&release);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    let mut holder = Holder {
        child: command.spawn()?,
        release,
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() {
        ensure!(
            holder.child.try_wait()?.is_none(),
            "Synthetic holder exited before readiness"
        );
        ensure!(
            Instant::now() < deadline,
            "Holder still starting; inspect PID {} before retrying",
            holder.child.id()
        );
        std::thread::sleep(Duration::from_millis(40));
    }
    let owners = devinspect::inspect_file(&file, &Progress::default())?;
    ensure!(
        owners
            .owners
            .iter()
            .any(|o| o.pid == holder.child.id() && o.process.identity == Identity::StartMatched),
        "Owned file holder PID/start identity was not reported"
    );
    ensure!(
        holder.child.try_wait()?.is_none(),
        "Inspector unexpectedly stopped the synthetic holder"
    );
    ensure!(
        fs::read(&file).is_err(),
        "Exclusive fixture unexpectedly released during inspection"
    );
    fs::write(&holder.release, b"release synthetic holder")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = holder.child.try_wait()? {
            ensure!(status.success(), "Synthetic holder exit failed");
            break;
        }
        ensure!(
            Instant::now() < deadline,
            "Holder still exiting; inspect PID {} before retrying",
            holder.child.id()
        );
        std::thread::sleep(Duration::from_millis(40));
    }
    ensure!(
        fs::read(&file)? == original,
        "Inspector changed target bytes"
    );
    let after = devinspect::inspect_file(&file, &Progress::default())?;
    ensure!(
        !after.owners.iter().any(|o| o.pid == holder.child.id()),
        "Exited holder still reported; refresh uncertainty requires investigation"
    );
    let result=format!("PASS: cold table query; owned TCP/UDP IPv4; {ipv6_evidence}\nPASS: exclusive Unicode-file holder PID/start matched, holder stayed running, target bytes unchanged, owner absent after normal fixture exit.\nNo connections, accepts, datagram sends, process kills, unlock/restart or firewall changes requested.\n");
    fs::write(root.join("result.txt"), &result)?;
    print!("{result}");
    println!("Synthetic evidence retained at {}", root.display());
    Ok(())
}

//! Firewall di Windows: senza una regola che consenta le connessioni in ingresso a POPMSG,
//! i messaggi passano (UDP) ma gli altri non riescono a scaricare i file che invii (TCP).

/// Stato della regola per questo eseguibile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fw {
    Ok,
    /// nessuna regola che consenta, o una che blocca
    Blocked,
    /// non verificabile (non Windows o errore)
    Unknown,
}

#[cfg(windows)]
fn ps(script: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(0x0800_0000)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(windows)]
fn exe() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Controlla le regole in ingresso per questo eseguibile (non serve essere amministratore).
#[cfg(windows)]
pub fn check() -> Fw {
    let script = format!(
        "$p = '{}'; Get-NetFirewallApplicationFilter -ErrorAction SilentlyContinue | \
         Where-Object {{ $_.Program -eq $p }} | Get-NetFirewallRule -ErrorAction SilentlyContinue | \
         Where-Object {{ $_.Enabled -eq 'True' -and $_.Direction -eq 'Inbound' }} | \
         ForEach-Object {{ Write-Output $_.Action }}",
        exe().replace('\'', "''")
    );
    let Some(out) = ps(&script) else { return Fw::Unknown };
    let actions: Vec<&str> = out.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    if actions.iter().any(|a| a.eq_ignore_ascii_case("Block")) || !actions.iter().any(|a| a.eq_ignore_ascii_case("Allow")) {
        Fw::Blocked
    } else {
        Fw::Ok
    }
}

#[cfg(not(windows))]
pub fn check() -> Fw {
    Fw::Unknown
}

/// Toglie le regole per questo eseguibile e ne aggiunge una che consente l'ingresso.
/// Chiede la conferma di amministratore (UAC).
#[cfg(windows)]
pub fn allow() -> Result<(), String> {
    let exe = exe();
    let cmd = format!(
        "netsh advfirewall firewall delete rule name=all program=\"{exe}\" & \
         netsh advfirewall firewall add rule name=POPMSG dir=in action=allow program=\"{exe}\" enable=yes profile=any"
    );
    let script = format!(
        "try {{ Start-Process -FilePath cmd.exe -ArgumentList '/c {}' -Verb RunAs -WindowStyle Hidden -Wait; Write-Output ok }} catch {{ Write-Output no }}",
        cmd.replace('\'', "''")
    );
    match ps(&script).as_deref().map(str::trim) {
        Some("ok") => Ok(()),
        _ => Err("Operazione annullata o non consentita.".into()),
    }
}

#[cfg(not(windows))]
pub fn allow() -> Result<(), String> {
    Err("Serve solo su Windows.".into())
}

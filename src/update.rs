//! Aggiornamenti: controlla se esiste una versione più nuova (release su GitHub
//! oppure una cartella di rete scelta nelle impostazioni) e sostituisce l'eseguibile.
//! Usa gli strumenti di sistema (curl, PowerShell) per non appesantire il programma.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const REPO: &str = "gbucciante/popmsg";

#[derive(Debug, Clone)]
pub struct Available {
    pub version: String,
    /// URL https oppure percorso del nuovo eseguibile
    pub source: String,
    /// dove scaricarlo a mano (pagina della release o cartella)
    pub page: String,
    /// installatore (Windows), usato se la cartella di POPMSG non è scrivibile
    pub setup: String,
}

fn cmd(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // nessuna finestra di console
    }
    c
}

/// Confronto di versioni "2.1.10" > "2.1.9".
pub fn newer(a: &str, b: &str) -> bool {
    let p = |s: &str| -> Vec<u64> { s.trim().trim_start_matches('v').split('.').map(|x| x.trim().parse().unwrap_or(0)).collect() };
    p(a) > p(b)
}

/// Nome dell'eseguibile da scaricare per questo sistema (None: aggiornamento solo a mano).
pub fn asset_name() -> Option<&'static str> {
    if cfg!(windows) {
        Some("POPMSG.exe")
    } else if cfg!(target_os = "linux") {
        Some("popmsg-linux")
    } else {
        None
    }
}

fn run_with_timeout(mut c: Command, secs: u64) -> Option<String> {
    let mut child = c.stdout(Stdio::piped()).stderr(Stdio::null()).stdin(Stdio::null()).spawn().ok()?;
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < end => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                return None;
            }
        }
    }
    let mut out = String::new();
    use std::io::Read;
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    Some(out)
}

/// Versione di un eseguibile di POPMSG (lo avvia con --version).
pub fn exe_version(path: &Path) -> Option<String> {
    let mut c = cmd(&path.to_string_lossy());
    c.arg("--version");
    let out = run_with_timeout(c, 15)?;
    let v = out.trim().trim_start_matches("POPMSG").trim().to_string();
    (!v.is_empty() && v.chars().next()?.is_ascii_digit()).then_some(v)
}

/// Cerca una versione più nuova di `current`.
pub fn check(current: &str, folder: &str) -> Result<Option<Available>, String> {
    let folder = folder.trim();
    if !folder.is_empty() {
        let name = asset_name().unwrap_or("POPMSG.exe");
        let p = Path::new(folder).join(name);
        if !p.exists() {
            return Err(format!("Nella cartella degli aggiornamenti non c'è {name}:\n{folder}"));
        }
        verify_signature(&p)?;
        let v = exe_version(&p).ok_or_else(|| format!("Impossibile leggere la versione di\n{}", p.display()))?;
        let setup = Path::new(folder).join("POPMSG-setup.exe").to_string_lossy().into_owned();
        return Ok(newer(&v, current).then(|| Available { version: v, source: p.to_string_lossy().into_owned(), page: folder.to_string(), setup }));
    }
    let api = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let mut c = cmd("curl");
    c.args(["-fsSL", "--max-time", "20", "-H", "Accept: application/vnd.github+json", "-A", "POPMSG", &api]);
    let json = run_with_timeout(c, 25).ok_or("Impossibile contattare GitHub (serve la connessione a Internet).")?;
    if json.trim().is_empty() {
        return Err("Nessuna versione pubblicata su GitHub.".into());
    }
    let tag = json
        .split("\"tag_name\"")
        .nth(1)
        .and_then(|r| r.split('"').nth(1))
        .ok_or("Risposta di GitHub non riconosciuta.")?
        .trim_start_matches('v')
        .to_string();
    let page = format!("https://github.com/{REPO}/releases/latest");
    let source = asset_name().map(|n| format!("https://github.com/{REPO}/releases/download/v{tag}/{n}")).unwrap_or_default();
    let setup = format!("https://github.com/{REPO}/releases/download/v{tag}/POPMSG-setup.exe");
    Ok(newer(&tag, current).then_some(Available { version: tag, source, page, setup }))
}

/// Su Windows, se questo POPMSG è firmato, il nuovo deve avere la stessa firma valida.
#[cfg(windows)]
pub fn verify_signature(new: &Path) -> Result<(), String> {
    let thumb = |p: &Path| -> Option<(String, String)> {
        let script = format!(
            "$s = Get-AuthenticodeSignature -LiteralPath '{}'; Write-Output $s.Status; Write-Output $s.SignerCertificate.Thumbprint",
            p.to_string_lossy().replace('\'', "''")
        );
        let mut c = cmd("powershell");
        c.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        let out = run_with_timeout(c, 30)?;
        let mut l = out.lines().map(|s| s.trim().to_string());
        Some((l.next().unwrap_or_default(), l.next().unwrap_or_default()))
    };
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let Some((_, mine)) = thumb(&me) else { return Ok(()) };
    if mine.is_empty() {
        return Ok(()); // questo POPMSG non è firmato: nessun confronto possibile
    }
    match thumb(new) {
        Some((st, t)) if t == mine && st == "Valid" => Ok(()),
        Some((st, t)) if t == mine => Err(format!("La firma della nuova versione non è valida ({st}).")),
        _ => Err("La nuova versione non è firmata con lo stesso certificato di POPMSG: aggiornamento annullato.".into()),
    }
}

#[cfg(not(windows))]
pub fn verify_signature(_new: &Path) -> Result<(), String> {
    Ok(())
}

fn old_path(exe: &Path) -> PathBuf {
    exe.with_file_name(if cfg!(windows) { "POPMSG.old.exe" } else { ".popmsg.old" })
}

/// Toglie il vecchio eseguibile lasciato da un aggiornamento.
pub fn cleanup() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(old_path(&exe));
    }
}

fn fetch(src: &str, dest: &Path) -> Result<(), String> {
    if src.starts_with("https://") {
        let mut c = cmd("curl");
        c.args(["-fsSL", "--max-time", "300", "-A", "POPMSG", "-o", &dest.to_string_lossy(), src]);
        let ok = c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
        if ok { Ok(()) } else { Err("Download della nuova versione non riuscito.".into()) }
    } else {
        std::fs::copy(src, dest).map(|_| ()).map_err(|e| format!("Copia non riuscita: {e}"))
    }
}

/// POPMSG installato in una cartella protetta (Programmi): si aggiorna con l'installatore,
/// che chiede i permessi di amministratore, chiude POPMSG e lo riavvia.
#[cfg(windows)]
fn run_setup(a: &Available) -> Result<PathBuf, String> {
    let dest = std::env::temp_dir().join("POPMSG-setup.exe");
    let _ = std::fs::remove_file(&dest);
    fetch(&a.setup, &dest)?;
    if let Err(e) = verify_signature(&dest) {
        let _ = std::fs::remove_file(&dest);
        return Err(e);
    }
    let d = dest.to_string_lossy().into_owned();
    cmd("cmd")
        .args(["/C", "start", "", &d, "/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART"])
        .spawn()
        .map_err(|e| format!("Impossibile avviare l'installatore: {e}"))?;
    Ok(PathBuf::new())
}

fn writable(dir: &Path) -> bool {
    let t = dir.join(".popmsg-prova");
    let ok = std::fs::write(&t, b"").is_ok();
    let _ = std::fs::remove_file(&t);
    ok
}

/// Scarica (o copia) la nuova versione, la controlla e sostituisce l'eseguibile.
/// Restituisce il percorso da avviare (vuoto: l'installatore è partito e riavvierà POPMSG).
pub fn apply(a: &Available) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    if !exe.parent().map(writable).unwrap_or(false) {
        return run_setup(a);
    }
    let new = exe.with_file_name(if cfg!(windows) { "POPMSG.new.exe" } else { ".popmsg.new" });
    let _ = std::fs::remove_file(&new);
    let no_write = |e: std::io::Error| {
        format!(
            "Impossibile scrivere nella cartella di POPMSG ({e}).\nAggiorna con l'installatore POPMSG-setup.exe:\n{}",
            a.page
        )
    };
    if !exe.parent().map(writable).unwrap_or(false) {
        return Err(no_write(std::io::Error::from(std::io::ErrorKind::PermissionDenied)));
    }
    if let Err(e) = fetch(&a.source, &new) {
        let _ = std::fs::remove_file(&new);
        return Err(e);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755));
    }
    let check = verify_signature(&new).and_then(|_| match exe_version(&new) {
        Some(v) if v == a.version => Ok(()),
        Some(v) => Err(format!("Il file scaricato è la versione {v}, non la {}.", a.version)),
        None => Err("Il file scaricato non è un POPMSG valido.".into()),
    });
    if let Err(e) = check {
        let _ = std::fs::remove_file(&new);
        return Err(e);
    }
    // su Windows l'eseguibile in uso si può rinominare ma non sovrascrivere
    let old = old_path(&exe);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(no_write)?;
    if let Err(e) = std::fs::rename(&new, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(no_write(e));
    }
    Ok(exe)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versioni() {
        assert!(newer("2.1.10", "2.1.9"));
        assert!(newer("v3.0.0", "2.9.9"));
        assert!(!newer("2.1.0", "2.1.0"));
        assert!(!newer("2.0.9", "2.1.0"));
    }
}

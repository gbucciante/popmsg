# Crea il certificato autofirmato con cui la compilazione firma POPMSG.exe.
# Da eseguire una sola volta, sul tuo PC: tasto destro -> "Esegui con PowerShell".
# La chiave privata resta solo nella cartella POPMSG-firma sul Desktop e nei Secrets di GitHub.

$pw = Read-Host "Scegli una password per il certificato (servira' su GitHub)" -AsSecureString
$cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject "CN=POPMSG, O=POPMSG" `
    -KeyAlgorithm RSA -KeyLength 3072 -HashAlgorithm SHA256 -KeyExportPolicy Exportable `
    -NotAfter (Get-Date).AddYears(10) -CertStoreLocation Cert:\CurrentUser\My
$dir = Join-Path ([Environment]::GetFolderPath('Desktop')) 'POPMSG-firma'
New-Item -ItemType Directory -Force $dir | Out-Null
Export-PfxCertificate -Cert $cert -FilePath "$dir\POPMSG-firma.pfx" -Password $pw | Out-Null
Export-Certificate -Cert $cert -FilePath "$dir\POPMSG-firma.cer" | Out-Null
[Convert]::ToBase64String([IO.File]::ReadAllBytes("$dir\POPMSG-firma.pfx")) |
    Set-Content -NoNewline -Encoding ascii "$dir\WINDOWS_CERT_PFX.txt"
# il certificato non serve nell'archivio personale: la copia e' nel file .pfx
Remove-Item "Cert:\CurrentUser\My\$($cert.Thumbprint)"

Write-Host ""
Write-Host "Fatto. Nella cartella $dir trovi:"
Write-Host "  WINDOWS_CERT_PFX.txt  -> contenuto da incollare nel Secret WINDOWS_CERT_PFX"
Write-Host "  POPMSG-firma.pfx      -> copia di sicurezza (tienila al sicuro)"
Write-Host "  POPMSG-firma.cer      -> certificato pubblico"
Write-Host "La password che hai scelto va nel Secret WINDOWS_CERT_PASSWORD."
Start-Process $dir
Read-Host "Premi Invio per chiudere"

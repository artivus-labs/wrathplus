# Native Windows bootstrap; does not require Make, WSL, or a POSIX shell.
[CmdletBinding()]
param(
    [ValidateSet('build', 'run', 'doctor', 'setup')][string]$Action = 'run',
    [string]$Data = '',
    [string]$Locale = 'enUS',
    [string]$Server = '127.0.0.1:3724',
    [string]$TargetDir = 'target'
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$cargo = Get-Command cargo.exe -ErrorAction SilentlyContinue
if ($cargo) { $cargoPath = $cargo.Source }
else {
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
    $cargoPath = Join-Path $cargoHome 'bin/cargo.exe'
    if (-not (Test-Path -LiteralPath $cargoPath -PathType Leaf)) {
        throw 'Cargo not found. Install Rust using rustup (https://rustup.rs) and Visual Studio C++ Build Tools + Windows SDK. See docs/CLIENT-SETUP.md.'
    }
}
if ($Action -eq 'setup' -and [string]::IsNullOrWhiteSpace($Data)) { throw 'setup requires -Data pointing to your own build-12340 Data folder.' }
Push-Location $repo
try {
    $target = [IO.Path]::GetFullPath($TargetDir)
    & $cargoPath build --locked -p wrathplus-manager --release --jobs 2 --target-dir $target
    if ($LASTEXITCODE -ne 0) { throw "Manager build failed ($LASTEXITCODE)." }
    if ($Action -ne 'build') {
        $manager = Join-Path $target 'release/wrathplus-manager.exe'
        $managerArgs = @('--repo', $repo, '--target-dir', $target)
        if ($Action -eq 'doctor') { $managerArgs += '--doctor' }
        if ($Action -eq 'setup') { $managerArgs += @('--setup', '--data', $Data, '--locale', $Locale, '--server', $Server) }
        & $manager @managerArgs
        if ($LASTEXITCODE -ne 0) { throw "Manager exited with code $LASTEXITCODE." }
    }
} finally { Pop-Location }

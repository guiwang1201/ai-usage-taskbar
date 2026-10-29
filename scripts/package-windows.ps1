param(
    [Parameter(Mandatory=$true)][string]$OutputRoot,
    [string]$BuildRoot
)
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$out = [IO.Path]::GetFullPath($OutputRoot)
if ($out.StartsWith($repo + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or $out -eq $repo -or $out -match '(?i)[\\/]OneDrive(?:[^\\/]*)[\\/]') { throw 'Use an output directory outside the repository and OneDrive.' }
$version = [regex]::Match([IO.File]::ReadAllText((Join-Path $repo 'Cargo.toml')), '(?m)^version = "([^"]+)"').Groups[1].Value
$target = 'x86_64-pc-windows-gnu'
if (-not $BuildRoot) {
    Push-Location $repo
    try {
        $metadata = & cargo metadata --offline --locked --no-deps --format-version 1
        if ($LASTEXITCODE -ne 0) { throw 'Could not resolve the Cargo build directory.' }
        $BuildRoot = ($metadata | ConvertFrom-Json).target_directory
    } finally { Pop-Location }
}
$build = [IO.Path]::GetFullPath($BuildRoot)
if ($build.StartsWith($repo + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or $build -eq $repo -or $build -match '(?i)[\\/]OneDrive(?:[^\\/]*)[\\/]') { throw 'Use a build directory outside the repository and OneDrive.' }
if ($build -eq $out -or $build.StartsWith($out + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or $out.StartsWith($build + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Build and package directories must be separate.' }
$package = Join-Path $out "IUsage-v$version-windows-x64"
if (Test-Path -LiteralPath $package) { throw 'Choose a fresh output directory; an existing package will not be overwritten.' }
$oldFlags = $env:RUSTFLAGS
try {
    $env:RUSTFLAGS = "--remap-path-prefix=$repo=source --remap-path-prefix=$env:USERPROFILE=user"
    Push-Location $repo
    try {
        & cargo test --offline --locked --target $target --target-dir $build
        if ($LASTEXITCODE -ne 0) { throw 'Offline tests failed.' }
        & cargo build --release --offline --locked --target $target --target-dir $build
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    } finally { Pop-Location }
    New-Item -ItemType Directory -Path $package -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $build "$target/release/iusage.exe") -Destination (Join-Path $package 'IUsage.exe')
    foreach ($file in @('LICENSE','PRIVACY.md','SECURITY.md','CHANGELOG.md')) { Copy-Item -LiteralPath (Join-Path $repo $file) -Destination $package }
    Copy-Item -LiteralPath (Join-Path $repo 'docs/QUICKSTART.md') -Destination $package
    $exe = Join-Path $package 'IUsage.exe'
    $binary = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($exe))
    if ($binary.Contains($env:USERPROFILE) -or $binary.Contains($repo)) { throw 'Personal build paths found in executable.' }
    $zip = "$package.zip"
    Compress-Archive -Path (Join-Path $package '*') -DestinationPath $zip
    $hashes = @(
        ((Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash + '  IUsage.exe')
        ((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash + "  $([IO.Path]::GetFileName($zip))")
    )
    $hashes | Set-Content -LiteralPath (Join-Path $out 'SHA256SUMS.txt') -Encoding ascii
    Write-Output "Package ready: $zip"
} finally { $env:RUSTFLAGS = $oldFlags }

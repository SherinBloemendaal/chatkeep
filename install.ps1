# Install chatkeep from GitHub releases.
# https://github.com/SherinBloemendaal/chatkeep
#Requires -Version 5.1

$ErrorActionPreference = 'Stop'

# --- presentation: logo, step lines, and a summary box ---------------------------------------
# chatkeep brand: teal, sky, violet (README banner and src/ui/banner.rs STOPS).
$script:Stops = @(@(45, 212, 191), @(56, 189, 248), @(167, 139, 250))
$script:Accent = '56;189;248'
$script:Tagline = 'Keep AI chat history attached to your projects.'
# Same letters as the CLI banner (src/ui/banner.rs). The file stays ASCII so Windows PowerShell 5.1
# reads it correctly without a BOM: # = full block, = | a b c d = the double-line box pieces.
$script:LogoBlock = @(
    ' ######b##b  ##b #####b ########b##b  ##b#######b#######b######b ',
    '##a====d##|  ##|##a==##bc==##a==d##| ##ad##a====d##a====d##a==##b',
    '##|     #######|#######|   ##|   #####ad #####b  #####b  ######ad',
    '##|     ##a==##|##a==##|   ##|   ##a=##b ##a==d  ##a==d  ##a===d ',
    'c######b##|  ##|##|  ##|   ##|   ##|  ##b#######b#######b##|     ',
    ' c=====dc=d  c=dc=d  c=d   c=d   c=d  c=dc======dc======dc=d     '
)
$script:LogoAscii = @(
    '  ____   _   _      _      _____   _  __  _____   _____   ____  ',
    ' / ___| | | | |    / \    |_   _| | |/ / | ____| | ____| |  _ \ ',
    '| |     | |_| |   / _ \     | |   | '' /  |  _|   |  _|   | |_) |',
    '| |___  |  _  |  / ___ \    | |   | . \  | |___  | |___  |  __/ ',
    ' \____| |_| |_| /_/   \_\   |_|   |_|\_\ |_____| |_____| |_|    '
)

$script:Esc = [char]27
$script:Vt = $false
try {
    $script:Vt = [bool]$Host.UI.SupportsVirtualTerminal
}
catch {
    $script:Vt = $false
}
if (-not [string]::IsNullOrEmpty($env:WT_SESSION) -or $PSVersionTable.PSVersion.Major -ge 7) {
    $script:Vt = $true
}
$script:Color = $script:Vt -and [string]::IsNullOrEmpty($env:NO_COLOR) -and -not [Console]::IsOutputRedirected
$script:Unicode = $false
if ($script:Vt) {
    try {
        [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
        $script:Unicode = $true
    }
    catch {
        $script:Unicode = $false
    }
}
$script:LogoBlock = $script:LogoBlock | ForEach-Object {
    $_.Replace('#', [char]0x2588).Replace('=', [char]0x2550).Replace('|', [char]0x2551).Replace('a', [char]0x2554).Replace('b', [char]0x2557).Replace('c', [char]0x255A).Replace('d', [char]0x255D)
}
if ($script:Unicode) {
    $script:MarkOk = [string][char]0x2713
    $script:MarkErr = [string][char]0x2717
    $script:Box = @([char]0x256D, [char]0x256E, [char]0x2570, [char]0x256F, [char]0x2500, [char]0x2502)
}
else {
    $script:MarkOk = '+'
    $script:MarkErr = 'x'
    $script:Box = @('+', '+', '+', '+', '-', '|')
}

function Format-ChatkeepStyle {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text,
        [Parameter(Mandatory = $true)][string]$Code
    )
    if (-not $script:Color -or $Text.Length -eq 0) {
        return $Text
    }
    return "$($script:Esc)[$($Code)m$Text$($script:Esc)[0m"
}

function Get-ChatkeepGradient {
    param([int]$Column, [int]$Span, [bool]$Shadow)
    $fraction = $Column / [Math]::Max(1, $Span - 1)
    if ($fraction -lt 0.5) {
        $from = $script:Stops[0]; $to = $script:Stops[1]; $local = $fraction * 2
    }
    else {
        $from = $script:Stops[1]; $to = $script:Stops[2]; $local = ($fraction - 0.5) * 2
    }
    $rgb = 0..2 | ForEach-Object { [int][Math]::Round($from[$_] + ($to[$_] - $from[$_]) * $local) }
    if ($Shadow) {
        $rgb = $rgb | ForEach-Object { [int][Math]::Round($_ * 0.55) }
        return "22;38;2;$($rgb[0]);$($rgb[1]);$($rgb[2])"
    }
    return "1;38;2;$($rgb[0]);$($rgb[1]);$($rgb[2])"
}

function Write-ChatkeepLogo {
    [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingWriteHost', '')]
    param()
    if ($script:Unicode) { $rows = $script:LogoBlock } else { $rows = $script:LogoAscii }
    $span = $rows[0].Length
    $columns = 80
    try {
        $columns = $Host.UI.RawUI.WindowSize.Width
    }
    catch {
        $columns = 80
    }
    Write-Host ''
    if ($span + 2 -le $columns) {
        foreach ($row in $rows) {
            if (-not $script:Color) {
                Write-Host "  $($row.TrimEnd())"
                continue
            }
            $builder = New-Object System.Text.StringBuilder
            [void]$builder.Append('  ')
            $last = ''
            for ($i = 0; $i -lt $row.Length; $i++) {
                $ch = $row[$i]
                if ($ch -eq ' ') {
                    [void]$builder.Append(' ')
                    continue
                }
                $code = Get-ChatkeepGradient -Column $i -Span $span -Shadow ($script:Unicode -and $ch -ne [char]0x2588)
                if ($code -ne $last) {
                    [void]$builder.Append("$($script:Esc)[$($code)m")
                    $last = $code
                }
                [void]$builder.Append($ch)
            }
            [void]$builder.Append("$($script:Esc)[0m")
            Write-Host $builder.ToString()
        }
        # Two blank lines under the art, like the CLI's own banner.
        Write-Host ''
        Write-Host ''
    }
    Write-Host "  $(Format-ChatkeepStyle -Text $script:Tagline -Code '1')  $(Format-ChatkeepStyle -Text 'installer' -Code '2')"
    Write-Host ''
}

function Write-ChatkeepStep {
    [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingWriteHost', '')]
    param(
        [Parameter(Mandatory = $true)][string]$Label,
        [Parameter(Mandatory = $true)][string]$Detail
    )
    $mark = Format-ChatkeepStyle -Text $script:MarkOk -Code '32'
    Write-Host ("  {0} {1} {2}" -f $mark, $Label.PadRight(10), (Format-ChatkeepStyle -Text $Detail -Code '2'))
}

function Write-ChatkeepPending {
    [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingWriteHost', '')]
    param([Parameter(Mandatory = $true)][string]$Label)
    if ($script:Color) {
        Write-Host -NoNewline ("  {0} {1}" -f (Format-ChatkeepStyle -Text '...' -Code "38;2;$($script:Accent)"), $Label)
    }
}

function Clear-ChatkeepPending {
    [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingWriteHost', '')]
    param()
    if ($script:Color) {
        Write-Host -NoNewline "`r$($script:Esc)[2K"
    }
}

function Write-ChatkeepFailure {
    [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingWriteHost', '')]
    param([Parameter(Mandatory = $true)][string]$Message)
    Clear-ChatkeepPending
    $mark = Format-ChatkeepStyle -Text $script:MarkErr -Code '1;31'
    Write-Host ("  {0} {1}" -f $mark, (Format-ChatkeepStyle -Text $Message -Code '31'))
}

function Get-ChatkeepDisplayPath {
    param([Parameter(Mandatory = $true)][string]$Path)
    $userHome = $env:USERPROFILE
    if (-not [string]::IsNullOrEmpty($userHome) -and $Path.StartsWith($userHome, [StringComparison]::OrdinalIgnoreCase)) {
        return '~' + $Path.Substring($userHome.Length)
    }
    return $Path
}

function Write-ChatkeepSummary {
    [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingWriteHost', '')]
    param([Parameter(Mandatory = $true)][object[]]$Lines)
    # Each line is @(plain, painted).
    $width = ($Lines | ForEach-Object { $_[0].Length } | Measure-Object -Maximum).Maximum
    $columns = 80
    try {
        $columns = $Host.UI.RawUI.WindowSize.Width
    }
    catch {
        $columns = 80
    }
    Write-Host ''
    if ($width + 10 -gt $columns) {
        foreach ($line in $Lines) {
            Write-Host "  $($line[1])"
        }
        Write-Host ''
        return
    }
    $edge = [string]$script:Box[4] * ($width + 6)
    $side = Format-ChatkeepStyle -Text ([string]$script:Box[5]) -Code '2'
    Write-Host ('  ' + (Format-ChatkeepStyle -Text ("$($script:Box[0])$edge$($script:Box[1])") -Code '2'))
    Write-Host ("  $side" + (' ' * ($width + 6)) + $side)
    foreach ($line in $Lines) {
        Write-Host ("  $side   " + $line[1] + (' ' * ($width - $line[0].Length)) + "   $side")
    }
    Write-Host ("  $side" + (' ' * ($width + 6)) + $side)
    Write-Host ('  ' + (Format-ChatkeepStyle -Text ("$($script:Box[2])$edge$($script:Box[3])") -Code '2'))
    Write-Host ''
}

function Get-ChatkeepVersion {
    param([string]$Raw)
    if ([string]::IsNullOrWhiteSpace($Raw)) {
        throw 'empty version'
    }
    if ($Raw -notmatch '^[A-Za-z0-9._-]+$') {
        throw "invalid version: $Raw"
    }
    if ($Raw.StartsWith('v')) {
        return $Raw
    }
    return "v$Raw"
}

function Save-ChatkeepFile {
    param(
        [Parameter(Mandatory = $true)][string]$Url,
        [Parameter(Mandatory = $true)][string]$Destination
    )
    $curl = Get-Command curl.exe -ErrorAction SilentlyContinue
    if ($null -ne $curl) {
        & curl.exe -fsSL --retry 3 --retry-delay 2 --proto '=https' --tlsv1.2 --output $Destination $Url
        if ($LASTEXITCODE -ne 0) {
            throw "download failed: $Url"
        }
        return
    }
    Invoke-WebRequest -Uri $Url -OutFile $Destination -UseBasicParsing
}

function Get-ChatkeepChecksum {
    param(
        [Parameter(Mandatory = $true)][string]$SumsPath,
        [Parameter(Mandatory = $true)][string]$Asset
    )
    foreach ($line in (Get-Content -Path $SumsPath)) {
        if ($line -match '^([0-9A-Fa-f]{64})\s+\*?(\S+)\s*$') {
            if ($Matches[2] -eq $Asset) {
                return $Matches[1].ToLowerInvariant()
            }
        }
    }
    throw "checksum mismatch: SHA256SUMS has no entry for $Asset"
}

try {
    if ($env:OS -ne 'Windows_NT') {
        throw 'unsupported platform: install.ps1 is for Windows. Use install.sh on macOS and Linux.'
    }

    $arch = $env:PROCESSOR_ARCHITECTURE
    if (-not [string]::IsNullOrEmpty($env:PROCESSOR_ARCHITEW6432)) {
        $arch = $env:PROCESSOR_ARCHITEW6432
    }
    if ($arch -ne 'AMD64') {
        throw "unsupported platform: windows $arch. chatkeep publishes windows x64 (x86_64-pc-windows-msvc)."
    }

    Write-ChatkeepLogo

    $repo = 'SherinBloemendaal/chatkeep'
    $github = "https://github.com/$repo"
    $target = 'x86_64-pc-windows-msvc'
    $asset = "chatkeep-$target.zip"

    if (-not [string]::IsNullOrWhiteSpace($env:CHATKEEP_VERSION)) {
        $version = Get-ChatkeepVersion $env:CHATKEEP_VERSION
    }
    else {
        $latestUrl = "$github/releases/latest"
        $effective = & curl.exe -fsSL --proto '=https' --tlsv1.2 -o NUL -w '%{url_effective}' $latestUrl
        if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($effective)) {
            throw "could not find the latest release at $latestUrl"
        }
        if ($effective -notmatch '/releases/tag/([^/?#]+)$') {
            throw "unexpected latest-release URL: $effective"
        }
        $version = Get-ChatkeepVersion $Matches[1]
    }

    if ([string]::IsNullOrWhiteSpace($env:CHATKEEP_INSTALL)) {
        $installDir = Join-Path $env:USERPROFILE '.chatkeep\bin'
    }
    else {
        $installDir = $env:CHATKEEP_INSTALL.TrimEnd('\')
    }
    New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    $installDir = (Resolve-Path -Path $installDir).Path

    Write-ChatkeepStep -Label 'Version' -Detail $version
    Write-ChatkeepStep -Label 'Platform' -Detail $target

    $tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("chatkeep-install-" + [System.Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force -Path $tempRoot | Out-Null
    try {
        $archive = Join-Path $tempRoot $asset
        $sums = Join-Path $tempRoot 'SHA256SUMS'
        $base = "$github/releases/download/$version"
        Write-ChatkeepPending -Label "Downloading $asset"
        Save-ChatkeepFile -Url "$base/$asset" -Destination $archive
        Save-ChatkeepFile -Url "$base/SHA256SUMS" -Destination $sums
        Clear-ChatkeepPending
        $size = (Get-Item -Path $archive).Length / 1MB
        Write-ChatkeepStep -Label 'Download' -Detail ("{0} ({1:N1} MB)" -f $asset, $size)

        $expected = Get-ChatkeepChecksum -SumsPath $sums -Asset $asset
        $actual = (Get-FileHash -Algorithm SHA256 -Path $archive).Hash.ToLowerInvariant()
        if ($actual -ne $expected) {
            throw "checksum mismatch for ${asset}: expected $expected actual $actual"
        }
        Write-ChatkeepStep -Label 'Checksum' -Detail 'SHA-256 matches SHA256SUMS'

        $extract = Join-Path $tempRoot 'extract'
        New-Item -ItemType Directory -Force -Path $extract | Out-Null
        Expand-Archive -Path $archive -DestinationPath $extract -Force
        $exe = Get-ChildItem -Path $extract -Filter 'chatkeep.exe' -Recurse -File | Select-Object -First 1
        if ($null -eq $exe) {
            throw 'archive does not contain chatkeep.exe'
        }

        $dest = Join-Path $installDir 'chatkeep.exe'
        $stage = Join-Path $installDir 'chatkeep.exe.new'
        Copy-Item -Force -Path $exe.FullName -Destination $stage
        if (Test-Path -Path $dest) {
            Remove-Item -Force -Path $dest
        }
        Move-Item -Force -Path $stage -Destination $dest
        Write-ChatkeepStep -Label 'Installed' -Detail (Get-ChatkeepDisplayPath -Path $dest)
    }
    finally {
        if (Test-Path -Path $tempRoot) {
            Remove-Item -Recurse -Force -Path $tempRoot
        }
    }

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $needle = $installDir.TrimEnd('\').ToLowerInvariant()
    $present = $false
    if (-not [string]::IsNullOrEmpty($userPath)) {
        foreach ($part in ($userPath -split ';')) {
            $item = $part.Trim().TrimEnd('\').ToLowerInvariant()
            if ($item -eq $needle) {
                $present = $true
            }
        }
    }
    if (-not $present) {
        if ([string]::IsNullOrEmpty($userPath)) {
            $newPath = $installDir
        }
        else {
            $newPath = "$installDir;$userPath"
        }
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
        Write-ChatkeepStep -Label 'PATH' -Detail 'added to the user PATH'
    }
    else {
        Write-ChatkeepStep -Label 'PATH' -Detail "already includes $(Get-ChatkeepDisplayPath -Path $installDir)"
    }

    if ([string]::IsNullOrEmpty($env:Path) -or ($env:Path.ToLowerInvariant() -notlike "*$needle*")) {
        $env:Path = "$installDir;$env:Path"
    }

    $installed = $version.TrimStart('v')
    $printed = & $dest --version 2>&1
    if ($LASTEXITCODE -eq 0) {
        $installed = ("$printed" -split ' ')[-1]
    }

    # Windows cannot change the PATH of terminals that are already open; this line loads it there.
    if ($installDir.StartsWith($env:USERPROFILE, [StringComparison]::OrdinalIgnoreCase)) {
        $load = '$env:Path += ";$HOME' + $installDir.Substring($env:USERPROFILE.Length) + '"'
    }
    else {
        $load = '$env:Path += ";' + $installDir + '"'
    }
    $title = "chatkeep $installed is installed"
    $run = Format-ChatkeepStyle -Text 'chatkeep' -Code "1;38;2;$($script:Accent)"
    Write-ChatkeepSummary -Lines @(
        , @("$($script:MarkOk) $title", "$(Format-ChatkeepStyle -Text $script:MarkOk -Code '1;32') $(Format-ChatkeepStyle -Text $title -Code '1')")
        , @('', '')
        , @("Location  $(Get-ChatkeepDisplayPath -Path $dest)", "$(Format-ChatkeepStyle -Text 'Location' -Code '2')  $(Get-ChatkeepDisplayPath -Path $dest)")
        , @('Next      open a new terminal and run chatkeep', "$(Format-ChatkeepStyle -Text 'Next' -Code '2')      open a new terminal and run $run")
        , @("Open tabs $load", "$(Format-ChatkeepStyle -Text 'Open tabs' -Code '2') $(Format-ChatkeepStyle -Text $load -Code "38;2;$($script:Accent)")")
    )
}
catch {
    Write-ChatkeepFailure -Message $_.Exception.Message
    exit 1
}

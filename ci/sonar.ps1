[CmdletBinding()]
param(
    [ValidateSet("up", "scan", "status", "down")]
    [string]$Action = "scan",

    [ValidateRange(0, 100)]
    [int]$CoverageThreshold = 80,

    [switch]$Volumes
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$composeFile = Join-Path $repoRoot "compose.sonar.yaml"
$sonarPort = if ($env:SONAR_PORT) { $env:SONAR_PORT } else { "9000" }
$sonarUrl = "http://127.0.0.1:$sonarPort"

function Assert-Command {
    param([string]$Name, [string]$Hint)
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Command '$Name' was not found. $Hint"
    }
}

function Invoke-Checked {
    param([string]$Command, [string[]]$Arguments)
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Command failed with exit code ${LASTEXITCODE}: $Command $($Arguments -join ' ')"
    }
}

function Write-ClippyReport {
    param([string[]]$Arguments, [string]$OutputPath)

    # Windows PowerShell's native `>` redirection writes UTF-16. Capture the
    # JSON lines and write UTF-8 without a BOM, as required by SonarQube.
    $lines = @(& cargo @Arguments)
    $exitCode = $LASTEXITCODE
    $utf8NoBom = [System.Text.UTF8Encoding]::new($false)
    [System.IO.File]::WriteAllLines($OutputPath, [string[]]$lines, $utf8NoBom)
    if ($exitCode -ne 0) {
        throw "Command failed with exit code ${exitCode}: cargo $($Arguments -join ' ')"
    }
}

function Start-Sonar {
    Assert-Command "docker" "Install Docker Desktop and enable Docker Compose v2."
    Invoke-Checked "docker" @("compose", "-f", $composeFile, "up", "-d")

    $deadline = (Get-Date).AddMinutes(3)
    while ((Get-Date) -lt $deadline) {
        try {
            $status = Invoke-RestMethod -Uri "$sonarUrl/api/system/status" -TimeoutSec 5
            if ($status.status -eq "UP") {
                Write-Host "SonarQube is ready: $sonarUrl"
                return
            }
        } catch {
            # The server is still starting.
        }
        Start-Sleep -Seconds 2
    }
    throw "SonarQube was not ready within 3 minutes. Check: docker compose -f compose.sonar.yaml logs sonarqube"
}

Set-Location $repoRoot

if ($Volumes -and $Action -ne "down") {
    throw "-Volumes is only valid with -Action down."
}

switch ($Action) {
    "up" {
        Start-Sonar
    }
    "status" {
        Assert-Command "docker" "Install Docker Desktop and enable Docker Compose v2."
        Invoke-Checked "docker" @("compose", "-f", $composeFile, "ps")
    }
    "down" {
        Assert-Command "docker" "Install Docker Desktop and enable Docker Compose v2."
        $downArguments = @("compose", "-f", $composeFile, "down")
        if ($Volumes) {
            $downArguments += "--volumes"
        }
        Invoke-Checked "docker" $downArguments
        if ($Volumes) {
            Write-Host "SonarQube stopped; named volumes and local analysis history were deleted."
        } else {
            Write-Host "SonarQube stopped; named volumes and analysis history were preserved."
        }
    }
    "scan" {
        Start-Sonar
        Assert-Command "cargo" "Install Rust through rustup."
        Assert-Command "git" "Install Git and run the scan from the repository checkout."

        $branch = (& git branch --show-current).Trim()
        if ($LASTEXITCODE -ne 0) {
            throw "Could not determine the current Git branch."
        }
        if ($branch -ne "master") {
            throw "Community Build only tracks the main branch. Switch to 'master' before scanning (current: '$branch')."
        }

        $scanner = Get-Command "sonar-scanner" -ErrorAction SilentlyContinue
        if (-not $scanner) {
            $scanner = Get-Command "sonar-scanner.bat" -ErrorAction SilentlyContinue
        }
        if (-not $scanner) {
            throw "sonar-scanner was not found. Install the official SonarScanner CLI and add its bin directory to PATH. The server is already available at $sonarUrl."
        }
        if ([string]::IsNullOrWhiteSpace($env:SONAR_TOKEN)) {
            throw "Set an analysis token: `$env:SONAR_TOKEN='<token>'. Create one at $sonarUrl under My Account -> Security."
        }

        Invoke-Checked "cargo" @("llvm-cov", "--version")
        $hostTriple = ((rustc -vV | Select-String "^host:").ToString().Split())[1]
        $reportDir = Join-Path $repoRoot "target\sonar"
        New-Item -ItemType Directory -Force $reportDir | Out-Null

        Invoke-Checked "cargo" @("fmt", "--all", "--check")
        Write-ClippyReport -Arguments @(
            "clippy", "--release", "--message-format=json", "--", "--no-deps"
        ) -OutputPath (Join-Path $reportDir "clippy-firmware.json")
        Write-ClippyReport -Arguments @(
            "clippy", "-p", "jzf407-logic", "--target", $hostTriple,
            "--all-targets", "--message-format=json"
        ) -OutputPath (Join-Path $reportDir "clippy-logic.json")
        Invoke-Checked "cargo" @(
            "llvm-cov", "-p", "jzf407-logic", "--target", $hostTriple,
            "--lcov", "--output-path", "target/sonar/lcov.info",
            "--remap-path-prefix", "--ignore-filename-regex", "tests",
            "--fail-under-lines", $CoverageThreshold.ToString()
        )
        Invoke-Checked "cargo" @(
            "llvm-cov", "report", "-p", "jzf407-logic", "--target", $hostTriple,
            "--summary-only", "--ignore-filename-regex", "tests"
        )

        $env:SONAR_HOST_URL = $sonarUrl
        Invoke-Checked $scanner.Source @()

        # Reports are uploaded first so findings remain visible in SonarQube;
        # these checks additionally keep the local no-warning policy blocking.
        Invoke-Checked "cargo" @("clippy", "--release", "--", "--no-deps", "-D", "warnings")
        Invoke-Checked "cargo" @("clippy", "-p", "jzf407-logic", "--target", $hostTriple, "--all-targets", "--", "-D", "warnings")
        Write-Host "Analysis completed: $sonarUrl/dashboard?id=jzf407-rust"
    }
}

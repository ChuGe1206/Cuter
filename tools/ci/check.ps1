# Everything that must pass before a commit.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\ci\check.ps1
#   ... -WithProbes      # also run the desktop probes (run_all.ps1 -Quick)
#
# It exists because "run clippy / tests / the i18n check" was three separate commands that had
# to be remembered, and forgetting one has already cost this project real bugs. Every step
# prints one line and the script exits non-zero on the first failure.
#
# ASCII only (see tools/probe/README.md for why).
param(
    [switch]$WithProbes,
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $root

# Resolve a Python interpreter once. Nothing here is tied to one machine: an explicit
# $env:CUTER_PYTHON wins, then whatever `python` is on PATH, then the `py` launcher, then the
# runtime that ships with the agent harness, then a per-user Python install.
#
# Existing is not enough of a test: on Windows `python` on PATH is often the Store alias stub,
# which exists, prints nothing and exits 9009. So every candidate has to answer `--version`.
function Resolve-Python {
    $candidates = @(
        $env:CUTER_PYTHON,
        (Get-Command python -ErrorAction SilentlyContinue | Select-Object -First 1).Source,
        (Get-Command py -ErrorAction SilentlyContinue | Select-Object -First 1).Source,
        (Join-Path $env:USERPROFILE '.dsh\dsh-runtimes\dsh-primary-runtime\dependencies\python\python.exe')
    ) | Where-Object { $_ -and (Test-Path $_) }
    $programs = Join-Path $env:LOCALAPPDATA 'Programs\Python'
    if (Test-Path $programs) {
        $candidates += @(Get-ChildItem $programs -Filter 'python.exe' -Recurse -ErrorAction SilentlyContinue |
            Select-Object -First 3 | ForEach-Object { $_.FullName })
    }
    foreach ($candidate in $candidates) {
        $previous = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            $version = & $candidate --version 2>&1 | Select-Object -First 1
            $code = $LASTEXITCODE
        } catch {
            $version = $null
            $code = -1
        } finally {
            $ErrorActionPreference = $previous
        }
        if ($code -eq 0 -and "$version" -match 'Python 3\.(1[1-9]|[2-9][0-9])') { return $candidate }
    }
    return $null
}

$script:python = Resolve-Python
if (-not $script:python) { throw 'no usable Python 3.11+ found; set CUTER_PYTHON to an interpreter' }
Write-Host ("== python: {0}" -f $script:python)

$script:failed = @()
function Step([string]$name, [scriptblock]$body) {
    Write-Host ("== {0}" -f $name)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    # cargo writes its progress to stderr; with ErrorActionPreference=Stop PowerShell would
    # treat that as a fatal error. The verdict is $LASTEXITCODE, not the stream.
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $body
    } finally {
        $ErrorActionPreference = $previous
    }
    $sw.Stop()
    if ($LASTEXITCODE -ne 0 -and $null -ne $LASTEXITCODE) {
        $script:failed += ("{0} (exit {1})" -f $name, $LASTEXITCODE)
        Write-Host ("   FAILED in {0:N1}s" -f $sw.Elapsed.TotalSeconds)
    } else {
        Write-Host ("   ok ({0:N1}s)" -f $sw.Elapsed.TotalSeconds)
    }
}

# 1. Lints. The project ships with zero warnings, so -D warnings is the gate.
Step 'clippy (workspace, -D warnings)' {
    cmd /c "cargo clippy --workspace --all-targets -- -D warnings 2>&1" | Select-Object -Last 3
}

# 2. Unit tests. These are all headless, so CI can run them too.
Step 'cargo test (workspace)' {
    cmd /c "cargo test --workspace 2>&1" | Select-String 'test result' | ForEach-Object { $_.Line }
}

# 3. i18n table completeness. `en_table.py` exits non-zero for UNUSED entries too (a known
# false positive: theme names are looked up through a runtime string), so the gate is the
# MISSING list only.
Step 'i18n table (no missing translations)' {
    $python = $script:python
    $env:PYTHONIOENCODING = 'utf-8'
    $out = & $python tools\i18n\en_table.py 2>&1
    $out | Select-String 'MISSING|UNUSED' | ForEach-Object { $_.Line }
    if ($out | Select-String 'MISSING') {
        $out | Select-String 'MISSING' -Context 0,20 | ForEach-Object { $_.Context.PostContext }
        throw 'missing translations'
    }
    $global:LASTEXITCODE = 0
}

# 3b. Documentation references. Docs rot silently: rename a file or renumber a section and the
# pointer stays, so the reader lands nowhere. check_refs.py treats every `path` and every
# "HANDOFF section N" reference as an assertion.
Step 'docs references (files and HANDOFF sections exist)' {
    $python = $script:python
    $env:PYTHONIOENCODING = 'utf-8'
    & $python tools\docs\check_refs.py
}

# 4. The release binary must build (that is what gets packaged).
if (-not $SkipBuild) {
    Step 'cargo build --release' {
        cmd /c "cargo build --release 2>&1" | Select-Object -Last 2
    }
}

# 5. Optional: the desktop probes. They need an interactive session (they synthesise real
# input and create real windows), so a headless CI runner CANNOT run them - that is why they
# are behind a switch here and absent from the GitHub workflow.
if ($WithProbes) {
    Step 'desktop probes (run_all.ps1 -Quick)' {
        & powershell -NoProfile -ExecutionPolicy Bypass -File tools\probe\run_all.ps1 -Quick
    }
}

Write-Host ''
if ($script:failed.Count -gt 0) {
    Write-Host ("FAILED: {0}" -f ($script:failed -join '; '))
    exit 1
}
Write-Host 'RESULT: PASS - clippy, tests, i18n and the release build are all green'

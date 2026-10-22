# Pick a Python 3 interpreter for the probes that need one (fake HTTP servers, OCR helpers).
#
#   . (Join-Path $PSScriptRoot 'python.ps1')
#   $Python = Require-Python -Why 'the fake OCR server'
#
# Existence is not enough of a test: on Windows `python` on PATH is often the Store alias
# stub, which exists, prints nothing and exits 9009. Every candidate therefore has to answer
# `--version`. Order: explicit -Preferred, then $env:CUTER_PYTHON, then PATH, then the py
# launcher, then the runtime shipped with the agent harness, then a per-user install.
#
# Skipping is a first-class outcome: a probe that cannot find an interpreter must report
# SKIPPED (exit 2) rather than pretend the product failed.
#
# ASCII only (see tools/probe/README.md).

function Resolve-Python {
    param([string]$Preferred = '')

    $candidates = @(
        $Preferred,
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
        if ($code -eq 0 -and "$version" -match 'Python 3') { return $candidate }
    }
    return $null
}

function Require-Python {
    param([string]$Preferred = '', [string]$Why = 'this probe')

    $exe = Resolve-Python -Preferred $Preferred
    if (-not $exe) {
        Write-Host "SKIPPED: no usable Python 3 found (set CUTER_PYTHON); $Why needs one"
        exit 2
    }
    return $exe
}

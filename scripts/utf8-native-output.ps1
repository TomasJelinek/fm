# Cargo prints UTF-8, and so does Python in UTF-8 mode (PYTHONUTF8=1 or -X utf8; the
# default from Python 3.15). PowerShell decodes captured native output with the console
# encoding, an OEM code page by default, which garbles a path below a non-ASCII
# directory such as a user profile. Run the command with UTF-8 decoding; the caller's
# console encoding is restored, since the code page belongs to the whole console.
function Invoke-Utf8NativeCommand([scriptblock]$Command) {
    $consoleEncoding = [Console]::OutputEncoding
    [Console]::OutputEncoding = New-Object Text.UTF8Encoding $false
    try {
        & $Command
    } finally {
        [Console]::OutputEncoding = $consoleEncoding
    }
}

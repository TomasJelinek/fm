$ErrorActionPreference = 'Stop'
# build.bat replaces its previous build deployment instead of accumulating complete
# installations, and removes only provably owned, unselected, idle installations.
$projectRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$deploy = Join-Path $projectRoot 'scripts/deploy-private.ps1'
$parent = if ($env:HOST_TEST_INSTALL_ROOT) { $env:HOST_TEST_INSTALL_ROOT } else { Join-Path $env:USERPROFILE '_build' }
New-Item -ItemType Directory -Path $parent -Force | Out-Null
$testRoot = Join-Path $parent ('build-deployment-retirement-test-' + [guid]::NewGuid().ToString('N'))
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
function Refuses([scriptblock]$Action, [string]$Reason) {
    $refused = $false
    try { & $Action | Out-Null } catch { $refused = $true }
    if (-not $refused) { throw "Expected refusal: $Reason" }
}
New-Item -ItemType Directory -Path $testRoot | Out-Null
$acl = New-Object Security.AccessControl.DirectorySecurity
$acl.SetSecurityDescriptorSddlForm("D:P(A;OICI;FA;;;$sid)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)",
    [Security.AccessControl.AccessControlSections]::Access)
Set-Acl -LiteralPath $testRoot -AclObject $acl
$originalProfile = $env:USERPROFILE
$junction = $null
$held = $null
$process = $null
$lock = $null
$waiting = $null
try {
    # Deployments go to the (isolated) profile root; registration stays in the fixture.
    $profileRoot = Join-Path $testRoot 'profile'
    $registry = Join-Path $testRoot 'registry'
    $source = Join-Path $testRoot 'source'
    New-Item -ItemType Directory -Path $profileRoot, $source | Out-Null
    [IO.File]::WriteAllText((Join-Path $source 'midnight-elevated-listing.exe'), 'authorization helper fixture')
    foreach ($name in @('basic', 'source', 'markdown')) {
        $package = Join-Path $source "viewers/midnight-viewer-$name"
        New-Item -ItemType Directory -Path "$package/notices" -Force | Out-Null
        foreach ($file in @('plugin.toml', "midnight_viewer_$name.dll", 'LICENSE', 'third-party.md', 'notices/LICENSE')) {
            [IO.File]::WriteAllText((Join-Path $package $file), "fixture $name $file")
        }
    }
    $script:build = 0
    function Deploy-Build {
        # Every build links a different launcher, so each has its own registration.
        $script:build++
        [IO.File]::WriteAllText((Join-Path $source 'midnight-explorer.exe'), "build $script:build")
        & $deploy -SourceDirectory $source -RegisterRuntime -RegistrationDirectory $registry -ReplaceBuildDeployment 3>&1 |
            Out-String -Width 4096
    }
    function Installations {
        return @(Get-ChildItem -LiteralPath $profileRoot -Directory | ForEach-Object Name |
            Where-Object { $_ -match '^MidnightExplorer-[0-9]{8}-[0-9]{6}-[0-9a-f]{8}$' } | Sort-Object)
    }
    function Selected {
        $hash = [regex]::Match([IO.File]::ReadAllText($script:pointer), 'registration_sha256 = "([0-9a-f]{64})"').Groups[1].Value
        $units = [regex]::Match([IO.File]::ReadAllText((Join-Path $registry "$hash.toml")),
            'executable_utf16 = \[([0-9,]+)\]').Groups[1].Value
        return Split-Path -Leaf (Split-Path -Parent (-join ($units.Split(',') | ForEach-Object { [char][uint16]$_ })))
    }
    $env:USERPROFILE = $profileRoot

    Refuses { & $deploy -SourceDirectory $source -ReplaceBuildDeployment } 'replacement without registration'
    Refuses { & $deploy -SourceDirectory $source -RegisterRuntime -RegistrationDirectory $registry -ReplaceBuildDeployment `
        -Destination (Join-Path $profileRoot 'explicit') } 'replacement of an explicit destination'
    Refuses { & $deploy -SourceDirectory $source -RegisterRuntime -RegistrationDirectory $registry -ReplaceBuildDeployment `
        -CreateDesktopShortcut } 'replacement of a shortcut installation'
    if (@(Get-ChildItem -LiteralPath $profileRoot -Force).Count) { throw 'A refused replacement created output' }

    $null = Deploy-Build
    $first = @(Installations)
    $pointers = @(Get-ChildItem -LiteralPath (Join-Path $registry 'launchers') -Filter '*.toml')
    if ($pointers.Count -ne 1) { throw 'Expected one launcher pointer' }
    $script:pointer = $pointers[0].FullName
    if ($first.Count -ne 1 -or (Selected) -cne $first[0]) { throw 'First build deployment is not registered' }
    $marker = [IO.File]::ReadAllText((Join-Path $profileRoot "$($first[0])/.midnight-build-deployment"))
    if ($marker -notmatch ('installation = "' + [regex]::Escape($first[0]) + '"')) { throw 'Ownership marker differs' }

    # Never owned by this build output: no marker (legacy), another output's marker,
    # a marker naming another directory (a copy) and an owned tree containing a link.
    $key = [regex]::Match($marker, 'launch_key = "([0-9a-f]{64})"').Groups[1].Value
    $planted = @{
        'MidnightExplorer-20200101-000000-0000000a' = $null
        'MidnightExplorer-20200101-000000-0000000b' = ('f' * 64), 'MidnightExplorer-20200101-000000-0000000b'
        'MidnightExplorer-20200101-000000-0000000c' = $key, $first[0]
        'MidnightExplorer-20200101-000000-0000000d' = $key, 'MidnightExplorer-20200101-000000-0000000d'
    }
    foreach ($name in $planted.Keys) {
        $directory = Join-Path $profileRoot $name
        New-Item -ItemType Directory -Path $directory | Out-Null
        [IO.File]::WriteAllText((Join-Path $directory 'midnight-explorer.exe'), 'user data')
        if ($planted[$name]) {
            [IO.File]::WriteAllText((Join-Path $directory '.midnight-build-deployment'),
                "schema_version = 1`nlaunch_key = `"$($planted[$name][0])`"`ninstallation = `"$($planted[$name][1])`"`n")
        }
    }
    New-Item -ItemType Directory -Path (Join-Path $testRoot 'link-target') | Out-Null
    $junction = Join-Path $profileRoot 'MidnightExplorer-20200101-000000-0000000d/redirect'
    New-Item -ItemType Junction -Path $junction -Target (Join-Path $testRoot 'link-target') | Out-Null

    # A running executable keeps its installation. A loaded image holds no open handle,
    # so the directory rename would succeed: only the write-open probe protects it.
    $running = Join-Path $profileRoot "$($first[0])/viewers/running-worker.exe"
    Copy-Item -LiteralPath (Join-Path $env:SystemRoot 'System32/PING.EXE') -Destination $running
    $process = Start-Process -FilePath $running -ArgumentList '-n', '600', '127.0.0.1' -WindowStyle Hidden -PassThru
    $writable = $true
    try { [IO.File]::Open($running, [IO.FileMode]::Open, [IO.FileAccess]::Write, [IO.FileShare]::ReadWrite).Dispose() }
    catch [IO.IOException] { $writable = $false }
    if ($writable -or $process.HasExited) { throw 'Precondition: the copied executable is not running' }
    # A file held open without delete sharing makes the rename fail instead.
    $opened = 'MidnightExplorer-20200101-000000-0000000f'
    New-Item -ItemType Directory -Path (Join-Path $profileRoot $opened) | Out-Null
    [IO.File]::WriteAllText((Join-Path $profileRoot "$opened/plugin.toml"), 'open plugin manifest')
    [IO.File]::WriteAllText((Join-Path $profileRoot "$opened/.midnight-build-deployment"),
        "schema_version = 1`nlaunch_key = `"$key`"`ninstallation = `"$opened`"`n")
    $held = [IO.File]::Open((Join-Path $profileRoot "$opened/plugin.toml"), [IO.FileMode]::Open,
        [IO.FileAccess]::Read, [IO.FileShare]::Read)
    $output = Deploy-Build
    $second = @(Installations | Where-Object { $_ -notin $first -and $_ -ne $opened -and -not $planted.ContainsKey($_) })
    if ($second.Count -ne 1 -or (Selected) -cne $second[0]) { throw 'Second build deployment is not registered' }
    foreach ($name in @($first[0], $opened)) {
        if (-not (Test-Path -LiteralPath (Join-Path $profileRoot "$name/.midnight-build-deployment"))) {
            throw "An installation in use was changed: $name"
        }
        if ($output -notmatch ('kept[^\r\n]*' + [regex]::Escape($name))) { throw "A kept installation was not reported: $name" }
    }
    if (-not (Test-Path -LiteralPath $running)) { throw 'The running executable was removed' }
    $process.Kill()
    if (-not $process.WaitForExit(30000)) { throw 'The running executable did not exit' }
    $process = $null
    $held.Dispose()
    $held = $null

    # An installation that another launcher pointer selects stays installed.
    $otherPointer = Join-Path (Split-Path -Parent $script:pointer) (('e' * 64) + '.toml')
    Copy-Item -LiteralPath $script:pointer -Destination $otherPointer
    $null = Deploy-Build
    $third = Selected
    foreach ($name in @($first[0], $opened)) {
        if (Test-Path -LiteralPath (Join-Path $profileRoot $name)) { throw "The idle superseded installation remained: $name" }
    }
    if (-not (Test-Path -LiteralPath (Join-Path $profileRoot $second[0]))) { throw 'A selected installation was removed' }
    Remove-Item -LiteralPath $otherPointer

    # Unrecognized registration state stops cleanup instead of guessing.
    [IO.File]::WriteAllText($otherPointer, 'unrecognized')
    $null = Deploy-Build
    $fourth = Selected
    foreach ($name in @($second[0], $third, $fourth)) {
        if (-not (Test-Path -LiteralPath (Join-Path $profileRoot $name))) { throw 'Cleanup guessed past unknown state' }
    }
    Remove-Item -LiteralPath $otherPointer

    # A partly deleted (retired) build installation is completed later.
    $retired = Join-Path $profileRoot 'MidnightExplorer-20200101-000000-0000000e.superseded'
    New-Item -ItemType Directory -Path "$retired/viewers" | Out-Null
    [IO.File]::WriteAllText("$retired/viewers/left.dll", 'left over')
    [IO.File]::WriteAllText("$retired/.midnight-build-deployment",
        "schema_version = 1`nlaunch_key = `"$key`"`ninstallation = `"MidnightExplorer-20200101-000000-0000000e`"`n")
    $null = Deploy-Build
    $fifth = Selected
    $remaining = @(Get-ChildItem -LiteralPath $profileRoot -Force | ForEach-Object Name | Sort-Object)
    $expected = @((@($planted.Keys) + $fifth) | Sort-Object)
    if (($remaining -join '|') -cne ($expected -join '|')) {
        throw "Unexpected remaining installations: $($remaining -join ', ')"
    }
    if ([IO.File]::ReadAllText((Join-Path $profileRoot 'MidnightExplorer-20200101-000000-0000000a/midnight-explorer.exe')) -cne 'user data') {
        throw 'An unowned directory was changed'
    }
    if (@(Get-ChildItem -LiteralPath $registry -Filter '*.toml').Count -ne 5) { throw 'Registration records changed' }

    # Deployments of one build output run one at a time: a second one waits, before it
    # creates its installation, while another holds the lock of this launch key.
    $lock = New-Object Threading.Mutex($false, "Local\MidnightBuildDeployment-$key")
    if (-not $lock.WaitOne(0)) { throw 'The build deployment lock is unexpectedly held' }
    $script:build++
    [IO.File]::WriteAllText((Join-Path $source 'midnight-explorer.exe'), "build $script:build")
    $start = New-Object Diagnostics.ProcessStartInfo((Get-Process -Id $PID).Path)
    $start.Arguments = "-NoLogo -NoProfile -NonInteractive -File `"$deploy`" -SourceDirectory `"$source`" " +
        "-RegisterRuntime -RegistrationDirectory `"$registry`" -ReplaceBuildDeployment"
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $waiting = [Diagnostics.Process]::Start($start)
    $errors = $waiting.StandardError.ReadToEndAsync()
    $deadline = [DateTime]::UtcNow.AddSeconds(60)
    do {
        $line = $waiting.StandardOutput.ReadLineAsync()
        $left = $deadline - [DateTime]::UtcNow
        if ($left -le [TimeSpan]::Zero -or -not $line.Wait($left)) { throw 'The second deployment did not wait for the lock' }
        if ($null -eq $line.Result) { throw "The second deployment ended without waiting: $($errors.Result)" }
    } until ($line.Result -match 'Waiting for another build deployment')
    # The child PowerShell may create AppData in this test's USERPROFILE.
    $remaining = @(Get-ChildItem -LiteralPath $profileRoot -Force | ForEach-Object Name |
        Where-Object { $_ -like 'MidnightExplorer-*' } | Sort-Object)
    if (($remaining -join '|') -cne ($expected -join '|')) {
        throw "A waiting deployment created an installation: $($remaining -join ', ')"
    }
    $lock.ReleaseMutex()
    $rest = $waiting.StandardOutput.ReadToEndAsync()
    if (-not $waiting.WaitForExit(120000)) { throw 'The waiting deployment did not finish' }
    if ($waiting.ExitCode -ne 0) { throw "The waiting deployment failed: $($rest.Result) $($errors.Result)" }
    $sixth = Selected
    if ($sixth -in $remaining) { throw 'The waiting deployment did not register its own installation' }
    if (Test-Path -LiteralPath (Join-Path $profileRoot $fifth)) { throw 'The waiting deployment kept the superseded installation' }
    Write-Output 'Build deployment retirement checks passed: refused combinations, running and open installations, selected, unrecognized-state, unowned, linked and retired trees, serialized deployments.'
} finally {
    if ($waiting -and -not $waiting.HasExited) { $waiting.Kill(); [void]$waiting.WaitForExit(30000) }
    if ($lock) {
        try { $lock.ReleaseMutex() } catch { }
        $lock.Dispose()
    }
    if ($process -and -not $process.HasExited) { $process.Kill(); [void]$process.WaitForExit(30000) }
    if ($held) { $held.Dispose() }
    $env:USERPROFILE = $originalProfile
    if ($junction -and (Test-Path -LiteralPath $junction)) { [IO.Directory]::Delete($junction) }
    $resolvedParent = [IO.Path]::GetFullPath($parent).TrimEnd('\')
    if ((Split-Path -Parent ([IO.Path]::GetFullPath($testRoot))) -cne $resolvedParent) { throw 'Unexpected fixture cleanup path' }
    Remove-Item -LiteralPath $testRoot -Recurse -Force
}

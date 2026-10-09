$ErrorActionPreference = 'Stop'
$source = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
$mouse = [regex]::Match($source, '(?s)WM_MOUSEACTIVATE\s*=>\s*\{(?<body>.*?)\n\s*WM_LBUTTONDOWN\s*=>')
if (-not $mouse.Success) { throw 'Missing mouse activation observer.' }
if ($mouse.Groups['body'].Value -notmatch 'DefWindowProcW\s*\(') {
    throw 'Mouse activation must use the default Windows behavior.'
}
if ($mouse.Groups['body'].Value -match 'LRESULT\s*\(\s*3\s*\)') {
    throw 'Old MA_NOACTIVATE override must be reverted.'
}
if ($source -match 'fn\s+mouse_noactivate_experiment_enabled\s*\(') {
    throw 'Old mouse activation opt-in must not remain enabled.'
}
Write-Host 'PASS: mouse noactivate experiment reverted; diagnostics remain.'

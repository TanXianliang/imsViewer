$ErrorActionPreference = "Stop"

if ($args.Count -eq 0) {
    throw "缺少要执行的命令"
}

$libclang = $null
if ($env:LIBCLANG_PATH -and (Test-Path (Join-Path $env:LIBCLANG_PATH "libclang.dll"))) {
    $libclang = $env:LIBCLANG_PATH
}

if (-not $libclang) {
    $candidates = @(
        (Join-Path $env:ProgramFiles "LLVM\bin"),
        (Join-Path $env:ProgramFiles "Microsoft Visual Studio\2022\Enterprise\VC\Tools\Llvm\x64\bin"),
        (Join-Path $env:ProgramFiles "Microsoft Visual Studio\2022\Professional\VC\Tools\Llvm\x64\bin"),
        (Join-Path $env:ProgramFiles "Microsoft Visual Studio\2022\Community\VC\Tools\Llvm\x64\bin"),
        (Join-Path $env:ProgramFiles "Microsoft Visual Studio\2022\BuildTools\VC\Tools\Llvm\x64\bin")
    )
    $libclang = $candidates |
        Where-Object { Test-Path (Join-Path $_ "libclang.dll") } |
        Select-Object -First 1
}

if (-not $libclang) {
    throw "未找到 libclang.dll，请安装 Visual Studio 的 C++ Clang 工具组件"
}

$env:LIBCLANG_PATH = $libclang
$command = $args[0]
$commandArgs = @($args | Select-Object -Skip 1)
& $command @commandArgs
exit $LASTEXITCODE

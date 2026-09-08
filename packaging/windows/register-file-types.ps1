# Registra o Yasmine como opção de "Abrir com" pra arquivos de áudio, com
# suporte a seleção múltipla — abrir vários arquivos de uma vez lança UMA
# instância com todos como argumento, não uma por arquivo (é o que
# `MultiSelectModel = Player` faz; é o mesmo valor que players como o VLC
# usam, e sem ele o Explorer abre um processo por arquivo selecionado).
#
# Só grava em HKEY_CURRENT_USER: não precisa administrador, não mexe em
# nada fora da conta do usuário atual, e dá pra desfazer apagando a chave
# "HKCU:\Software\Classes\Applications\yasmine.exe".
#
# Uso: extraia o .zip e rode (duplo clique em register-file-types.cmd, ou
# `powershell -ExecutionPolicy Bypass -File register-file-types.ps1`).

$ErrorActionPreference = "Stop"

$exe = Join-Path $PSScriptRoot "yasmine.exe"
if (-not (Test-Path $exe)) {
    Write-Error "yasmine.exe não está ao lado deste script — extraia o .zip inteiro antes de rodar."
    exit 1
}
$exe = (Resolve-Path $exe).Path

$appKey = "HKCU:\Software\Classes\Applications\yasmine.exe"
New-Item -Path "$appKey\shell\open\command" -Force | Out-Null
Set-ItemProperty -Path "$appKey\shell\open\command" -Name "(default)" -Value "`"$exe`" %1"
Set-ItemProperty -Path "$appKey\shell\open" -Name "MultiSelectModel" -Value "Player"

$extensoes = ".mp3", ".flac", ".m4a", ".mp4", ".aac", ".ogg", ".oga", ".opus", ".wav", ".wave", ".aiff", ".aif"
foreach ($ext in $extensoes) {
    $progidsKey = "HKCU:\Software\Classes\$ext\OpenWithProgids"
    New-Item -Path $progidsKey -Force | Out-Null
    Set-ItemProperty -Path $progidsKey -Name "Applications\yasmine.exe" -Value "" -Type String
}

Write-Host "Pronto. Botao direito num arquivo de audio -> Abrir com -> Yasmine."
Write-Host "Selecionar varios arquivos e abrir todos de uma vez tambem funciona."

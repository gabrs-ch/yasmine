@echo off
rem Atalho pro register-file-types.ps1 — evita a tela de permissão de
rem execução de script que o PowerShell mostra por padrão num .ps1 clicado
rem direto. Duplo clique nisto basta.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0register-file-types.ps1"
pause

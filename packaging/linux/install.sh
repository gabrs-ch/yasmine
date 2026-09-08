#!/bin/sh
# Instala o Yasmine pro usuário atual: binário em ~/.local/bin, ícone no
# tema de ícones do usuário, entrada em ~/.local/share/applications. Sem
# root, sem sudo — não mexe em nada fora de $HOME.
#
# Depois de rodar, "Yasmine" aparece no menu de aplicativos e no "Abrir
# com" de qualquer arquivo de áudio (mp3, flac, m4a, ogg, opus, wav, aiff).
#
# Uso: extraia o .tar.gz e rode este script de dentro da pasta extraída.
#   ./install.sh

set -eu

aqui="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"

bin_dir="$HOME/.local/bin"
icon_dir="$HOME/.local/share/icons/hicolor/256x256/apps"
app_dir="$HOME/.local/share/applications"

mkdir -p "$bin_dir" "$icon_dir" "$app_dir"

install -m 755 "$aqui/yasmine" "$bin_dir/yasmine"
install -m 644 "$aqui/icon-256.png" "$icon_dir/yasmine.png"
install -m 644 "$aqui/yasmine.desktop" "$app_dir/yasmine.desktop"

# Atualiza o índice de aplicativos, se a ferramenta existir — sem ela a
# entrada só aparece depois do próximo login, o que é enganoso logo depois
# de instalar.
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$app_dir" 2>/dev/null || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
fi

case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *)
        echo "Aviso: $bin_dir não está no PATH — 'yasmine' não vai rodar direto"
        echo "do terminal (o atalho no menu de aplicativos funciona do mesmo jeito)."
        echo "Pra rodar do terminal também, adicione ao seu shell rc:"
        echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
        ;;
esac

echo "Instalado. Yasmine já aparece no menu de aplicativos e no \"Abrir com\" de arquivos de áudio."

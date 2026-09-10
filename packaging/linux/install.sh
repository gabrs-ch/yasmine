#!/bin/sh
# Instala o Yasmine pro usuário atual: binário em ~/.local/bin, ícone e
# entrada de menu em ~/.local/share. Sem root, sem sudo — não mexe em nada
# fora de $HOME.
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

# Limpa cópias antigas do ícone antes de instalar a nova. `Icon=` por NOME
# (o que o .desktop usava antes) faz a busca no tema achar qualquer
# `yasmine.png` que uma instalação anterior tenha deixado noutro tamanho ou
# noutra pasta — e o menu fica mostrando o ícone velho mesmo com o arquivo
# certo no lugar. Some com todos e deixa só o de agora.
for d in "$HOME/.local/share/icons" "$HOME/.icons" "$HOME/.local/share/pixmaps"; do
    [ -d "$d" ] && find "$d" -type f -iname 'yasmine.*' -delete 2>/dev/null || true
done
install -m 644 "$aqui/icon-256.png" "$icon_dir/yasmine.png"

# O .desktop instalado aponta pro ícone por CAMINHO ABSOLUTO, não pelo nome
# `yasmine` — sem a busca no tema, não tem como cair num arquivo velho.
sed "s|^Icon=.*|Icon=$icon_dir/yasmine.png|" "$aqui/yasmine.desktop" \
    > "$app_dir/yasmine.desktop"
chmod 644 "$app_dir/yasmine.desktop"

# Força o cache de ícones e o índice do menu a reler — sem `-f`/`forceupdate`
# o ambiente costuma achar que nada mudou e o ícone velho fica.
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
fi
if command -v xdg-desktop-menu >/dev/null 2>&1; then
    xdg-desktop-menu forceupdate 2>/dev/null || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$app_dir" 2>/dev/null || true
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
echo "Se o menu ainda mostrar o ícone antigo: no XFCE, \"xfce4-panel -r\"; no GNOME/KDE, deslogar e logar."

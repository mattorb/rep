#!/usr/bin/env bash
set -euo pipefail

# Tools whose user-level config dirs should receive skill symlinks.
TOOLS=(claude codex gemini opencode hermes droid)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SKILLS_SRC="$SCRIPT_DIR/.agents/skills"

if [ ! -d "$SKILLS_SRC" ]; then
    echo "No skills directory found at $SKILLS_SRC" >&2
    exit 1
fi

shopt -s nullglob
skills=("$SKILLS_SRC"/*/)
shopt -u nullglob

if [ ${#skills[@]} -eq 0 ]; then
    echo "No skills found in $SKILLS_SRC" >&2
    exit 0
fi

confirm() {
    local prompt="$1"
    local reply

    while true; do
        printf "%s [y/N] " "$prompt"
        if ! read -r reply; then
            echo
            return 1
        fi
        if [ ! -t 0 ]; then
            echo
        fi

        case "$reply" in
            [Yy] | [Yy][Ee][Ss])
                return 0
                ;;
            "" | [Nn] | [Nn][Oo])
                return 1
                ;;
            *)
                echo "Please answer y or n."
                ;;
        esac
    done
}

echo "This script installs skills by creating symlinks."
echo
echo "Source skills directory:"
echo "  $SKILLS_SRC"
echo
echo "Skills found:"
for skill_path in "${skills[@]}"; do
    skill_path="${skill_path%/}"
    echo "  - $(basename "$skill_path")"
done
echo
echo "Destination skill directories:"
for tool in "${TOOLS[@]}"; do
    echo "  - $HOME/.$tool/skills"
done
echo
echo "You will be asked before each symlink is created or updated."
echo "For each approved symlink, its destination directory will be created if needed."
echo

for tool in "${TOOLS[@]}"; do
    dest_dir="$HOME/.$tool/skills"

    for skill_path in "${skills[@]}"; do
        skill_path="${skill_path%/}"
        skill_name="$(basename "$skill_path")"
        link="$dest_dir/$skill_name"

        echo "Planned symlink:"
        echo "  Link:   $link"
        echo "  Target: $skill_path"

        if [ -L "$link" ]; then
            echo "  Existing: symlink -> $(readlink "$link")"
        elif [ -e "$link" ]; then
            echo "  Existing: non-symlink path at $link"
        else
            echo "  Existing: none"
        fi

        if confirm "Create or update this symlink?"; then
            mkdir -p "$dest_dir"
            ln -sfn "$skill_path" "$link"
            echo "Created: $link -> $skill_path"
        else
            echo "Skipped: $link"
        fi
        echo
    done
done

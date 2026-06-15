#!/usr/bin/env bash
set -euo pipefail

app_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
venv_dir="$app_dir/.venv"
prepare_dir="$app_dir/assets/prepare"
requirements_file="$prepare_dir/requirements.txt"
proj57_clone_dir="$app_dir/third_party/proj57"

if [[ ! -f "$requirements_file" ]]; then
    echo "error: requirements file not found: $requirements_file" >&2
    exit 1
fi

if [[ ! -d "$proj57_clone_dir/.git" ]]; then
    mkdir -p "$app_dir/third_party"
    git clone git@github.com:chenlongos/proj57.git "$proj57_clone_dir"
else
    echo "info: proj57 already exists: $proj57_clone_dir"
fi

if command -v uv >/dev/null 2>&1; then
    if [[ ! -d "$venv_dir" ]]; then
        uv venv "$venv_dir"
    fi
    uv pip install --python "$venv_dir/bin/python" -r "$requirements_file"
else
    if [[ ! -d "$venv_dir" ]]; then
        python3 -m venv "$venv_dir"
    fi
    "$venv_dir/bin/pip" install -r "$requirements_file"
fi

echo "info: python env ready: $venv_dir"
echo "info: proj57 clone ready: $proj57_clone_dir"
echo "info: activate with: source $venv_dir/bin/activate"

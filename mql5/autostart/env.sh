# shellcheck shell=bash
# Load the repository-root .env without executing it.
#
# The Rust binaries use dotenvy; this helper keeps the shell scripts on the same
# single .env. Values are taken literally (no expansion, no command
# substitution), and only variables that are not already set in the environment
# are loaded, so explicit overrides always win.
#
# Usage: load_mt5_env /path/to/.env

load_mt5_env() {
    local env_file="${1:-}"
    [[ -n "$env_file" && -f "$env_file" ]] || return 0

    local line key value first last
    while IFS= read -r line || [[ -n "$line" ]]; do
        line="${line%$'\r'}"
        # Strip leading whitespace.
        line="${line#"${line%%[![:space:]]*}"}"

        if [[ -z "$line" || "$line" == \#* ]]; then
            continue
        fi

        # Accept the common `export KEY=value` form.
        if [[ "$line" == export[[:space:]]* ]]; then
            line="${line#export}"
            line="${line#"${line%%[![:space:]]*}"}"
        fi

        if [[ "$line" != *=* ]]; then
            continue
        fi

        key="${line%%=*}"
        value="${line#*=}"
        # Trim trailing whitespace from the key.
        key="${key%"${key##*[![:space:]]}"}"

        # Strip matching surrounding quotes.
        if [[ ${#value} -ge 2 ]]; then
            first="${value:0:1}"
            last="${value: -1}"
            if [[ "$first" == '"' && "$last" == '"' ]] ||
               [[ "$first" == "'" && "$last" == "'" ]]; then
                value="${value:1:${#value}-2}"
            fi
        fi

        [[ "$key" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]] || continue

        # Do not override an explicitly exported value.
        if ! declare -p "$key" >/dev/null 2>&1; then
            export "$key=$value"
        fi
    done < "$env_file"
}

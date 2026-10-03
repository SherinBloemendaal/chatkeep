#!/usr/bin/env bash
# Install chatkeep from GitHub releases.
# https://github.com/SherinBloemendaal/chatkeep

if [ -z "${BASH_VERSION:-}" ]; then
  echo "error: install.sh requires bash" >&2
  exit 1
fi

set -euo pipefail

repo="SherinBloemendaal/chatkeep"
github="https://github.com/${repo}"

# chatkeep brand: teal, sky, violet (README banner and src/ui/banner.rs STOPS).
stops=(45 212 191 56 189 248 167 139 250)
basic_codes=(36 34 35)
accent_rgb="56;189;248"
accent_256=39
tagline="Keep AI chat history attached to your projects."
# Same letters as the CLI banner (src/ui/banner.rs).
logo_block=(
  ' ██████╗██╗  ██╗ █████╗ ████████╗██╗  ██╗███████╗███████╗██████╗ '
  '██╔════╝██║  ██║██╔══██╗╚══██╔══╝██║ ██╔╝██╔════╝██╔════╝██╔══██╗'
  '██║     ███████║███████║   ██║   █████╔╝ █████╗  █████╗  ██████╔╝'
  '██║     ██╔══██║██╔══██║   ██║   ██╔═██╗ ██╔══╝  ██╔══╝  ██╔═══╝ '
  '╚██████╗██║  ██║██║  ██║   ██║   ██║  ██╗███████╗███████╗██║     '
  ' ╚═════╝╚═╝  ╚═╝╚═╝  ╚═╝   ╚═╝   ╚═╝  ╚═╝╚══════╝╚══════╝╚═╝     '
)
logo_ascii=(
  '  ____   _   _      _      _____   _  __  _____   _____   ____  '
  ' / ___| | | | |    / \    |_   _| | |/ / | ____| | ____| |  _ \ '
  '| |     | |_| |   / _ \     | |   | '\'' /  |  _|   |  _|   | |_) |'
  '| |___  |  _  |  / ___ \    | |   | . \  | |___  | |___  |  __/ '
  ' \____| |_| |_| /_/   \_\   |_|   |_|\_\ |_____| |_____| |_|    '
)

# Presentation: logo, step lines, spinner, and a summary box. Runs on bash 3.2 (macOS).
tty=0
if [ -t 1 ]; then
  tty=1
  # Keep the terminal on fd 3: inside $(...) stdout is a pipe, and with curl | bash so is stdin.
  exec 3>&1
fi

color=1
if [ -n "${NO_COLOR:-}" ] || [ "${TERM:-}" = "dumb" ] || { [ "$tty" -eq 0 ] && [ -z "${CLICOLOR_FORCE:-}" ]; }; then
  color=0
fi

# Unicode art needs a UTF-8 locale and a bash that counts characters, not bytes.
unicode=0
probe="✓"
case "${LC_ALL:-${LC_CTYPE:-${LANG:-}}}" in
  *[Uu][Tt][Ff]-8* | *[Uu][Tt][Ff]8*)
    if [ "${#probe}" -eq 1 ]; then
      unicode=1
    fi
    ;;
  "")
    # No locale at all is common in fresh macOS shells. Try a UTF-8 one for this script only;
    # an explicit C or POSIX locale keeps the ASCII fallback.
    for candidate in en_US.UTF-8 C.UTF-8; do
      LC_CTYPE="$candidate"
      if [ "${#probe}" -eq 1 ]; then
        unicode=1
        break
      fi
    done
    if [ "$unicode" -eq 0 ]; then
      unset LC_CTYPE
    fi
    ;;
esac

depth="basic"
case "${COLORTERM:-}" in
  truecolor | 24bit) depth="truecolor" ;;
  *)
    case "${TERM:-}" in
      *256color*) depth="256" ;;
    esac
    ;;
esac

if [ "$color" -eq 1 ]; then
  c_reset=$'\033[0m'
  c_bold=$'\033[1m'
  c_dim=$'\033[2m'
  c_red=$'\033[31m'
  c_green=$'\033[32m'
  c_yellow=$'\033[33m'
  case "$depth" in
    truecolor) c_accent=$'\033['"38;2;${accent_rgb}m" ;;
    256) c_accent=$'\033['"38;5;${accent_256}m" ;;
    *) c_accent=$'\033['"${basic_codes[1]}m" ;;
  esac
else
  c_reset=""
  c_bold=""
  c_dim=""
  c_red=""
  c_green=""
  c_yellow=""
  c_accent=""
fi

if [ "$unicode" -eq 1 ]; then
  mark_ok="✓"
  mark_warn="!"
  mark_err="✗"
  spinner_frames=("⠋" "⠙" "⠹" "⠸" "⠼" "⠴" "⠦" "⠧" "⠇" "⠏")
  box_tl="╭"
  box_tr="╮"
  box_bl="╰"
  box_br="╯"
  box_h="─"
  box_v="│"
  logo_rows=("${logo_block[@]}")
else
  mark_ok="+"
  mark_warn="!"
  mark_err="x"
  spinner_frames=("|" "/" "-" "\\")
  box_tl="+"
  box_tr="+"
  box_bl="+"
  box_br="+"
  box_h="-"
  box_v="|"
  logo_rows=("${logo_ascii[@]}")
fi

cursor_hidden=0

terminal_columns() {
  local size="" cols=""
  if [ "$tty" -eq 1 ]; then
    size="$(stty size <&3 2>/dev/null || true)"
    cols="${size#* }"
  fi
  case "$cols" in
    "" | *[!0-9]*) cols="${COLUMNS:-80}" ;;
  esac
  case "$cols" in
    "" | *[!0-9]*) cols=80 ;;
  esac
  printf '%s\n' "$cols"
}

# Sets $paint to the escape for column $1 of $2 on the logo gradient ($stops, three RGB stops).
gradient_escape() {
  local column="$1" span="$2" shadow="$3"
  local fraction=$((column * 1000 / (span > 1 ? span - 1 : 1)))
  local from_r from_g from_b to_r to_g to_b local_f
  if [ "$fraction" -lt 500 ]; then
    from_r=${stops[0]} from_g=${stops[1]} from_b=${stops[2]} to_r=${stops[3]} to_g=${stops[4]} to_b=${stops[5]}
    local_f=$((fraction * 2))
  else
    from_r=${stops[3]} from_g=${stops[4]} from_b=${stops[5]} to_r=${stops[6]} to_g=${stops[7]} to_b=${stops[8]}
    local_f=$(((fraction - 500) * 2))
  fi
  local r=$((from_r + (to_r - from_r) * local_f / 1000))
  local g=$((from_g + (to_g - from_g) * local_f / 1000))
  local b=$((from_b + (to_b - from_b) * local_f / 1000))
  local weight="1"
  if [ "$shadow" -eq 1 ]; then
    r=$((r * 55 / 100))
    g=$((g * 55 / 100))
    b=$((b * 55 / 100))
    weight="22"
  fi
  case "$depth" in
    truecolor)
      paint=$'\033['"${weight};38;2;${r};${g};${b}m"
      ;;
    256)
      local lr lg lb
      lr=$(xterm_level "$r")
      lg=$(xterm_level "$g")
      lb=$(xterm_level "$b")
      paint=$'\033['"${weight};38;5;$((16 + 36 * lr + 6 * lg + lb))m"
      ;;
    *)
      local code="${basic_codes[0]}"
      if [ "$fraction" -ge 670 ]; then
        code="${basic_codes[2]}"
      elif [ "$fraction" -ge 340 ]; then
        code="${basic_codes[1]}"
      fi
      if [ "$shadow" -eq 1 ]; then
        paint=$'\033[2;'"${code}m"
      else
        paint=$'\033[1;'"${code}m"
      fi
      ;;
  esac
}

xterm_level() {
  if [ "$1" -lt 48 ]; then
    printf '0'
  elif [ "$1" -lt 115 ]; then
    printf '1'
  else
    printf '%s' $((($1 - 35) / 40))
  fi
}

print_logo() {
  local span="${#logo_rows[0]}"
  local cols
  cols="$(terminal_columns)"
  printf '\n'
  if [ $((span + 2)) -le "$cols" ]; then
    local row line i ch shadow last=""
    for row in "${logo_rows[@]}"; do
      if [ "$color" -eq 0 ]; then
        printf '  %s\n' "$row"
        continue
      fi
      line=""
      i=0
      while [ "$i" -lt "${#row}" ]; do
        ch="${row:$i:1}"
        if [ "$ch" = " " ]; then
          line="${line} "
        else
          shadow=0
          if [ "$unicode" -eq 1 ] && [ "$ch" != "█" ]; then
            shadow=1
          fi
          gradient_escape "$i" "$span" "$shadow"
          if [ "$paint" != "$last" ]; then
            line="${line}${paint}"
            last="$paint"
          fi
          line="${line}${ch}"
        fi
        i=$((i + 1))
      done
      last=""
      printf '  %s%s\n' "$line" "$c_reset"
    done
    # Two blank lines under the art, like the CLI's own banner.
    printf '\n\n'
  fi
  printf '  %s%s%s  %s\n\n' "$c_bold" "$tagline" "$c_reset" "${c_dim}installer${c_reset}"
}

step() {
  printf '  %s %-10s %s\n' "${c_green}${mark_ok}${c_reset}" "$1" "${c_dim}$2${c_reset}"
}

note() {
  printf '  %s %-10s %s\n' "${c_yellow}${mark_warn}${c_reset}" "$1" "$2"
}

error() {
  printf '  %s %s\n' "${c_bold}${c_red}${mark_err}${c_reset}" "${c_red}$*${c_reset}" >&2
}

die() {
  error "$@"
  exit 1
}

show_cursor() {
  if [ "$cursor_hidden" -eq 1 ]; then
    printf '\033[?25h'
    cursor_hidden=0
  fi
}

# Runs "$@" in the background with a spinner and label $1 when stdout is a terminal.
# Its stderr is held back and printed after the spinner line is cleared.
spin() {
  local label="$1"
  shift
  if [ "$tty" -eq 0 ]; then
    "$@"
    return
  fi
  # A label wider than the terminal would wrap, and \r would only redraw its last row.
  local room=$(($(terminal_columns) - 5))
  if [ "${#label}" -gt "$room" ] && [ "$room" -gt 3 ]; then
    label="${label:0:$((room - 3))}..."
  fi
  local log="${tmpdir}/spin.log"
  "$@" 2>"$log" &
  local pid=$!
  local frame=0
  local count="${#spinner_frames[@]}"
  printf '\033[?25l'
  cursor_hidden=1
  while kill -0 "$pid" 2>/dev/null; do
    printf '\r  %s %s' "${c_accent}${spinner_frames[$((frame % count))]}${c_reset}" "$label"
    frame=$((frame + 1))
    sleep 0.08
  done
  printf '\r\033[2K'
  show_cursor
  local status=0
  wait "$pid" || status=$?
  if [ "$status" -ne 0 ] && [ -s "$log" ]; then
    sed 's/^/    /' "$log" >&2
  fi
  return "$status"
}

display_path() {
  case "$1" in
    "${HOME}"/*) printf '~%s\n' "${1#"${HOME}"}" ;;
    *) printf '%s\n' "$1" ;;
  esac
}

human_size() {
  awk -v bytes="$1" 'BEGIN {
    if (bytes >= 1048576) printf "%.1f MB", bytes / 1048576
    else if (bytes >= 1024) printf "%.0f KB", bytes / 1024
    else printf "%d B", bytes
  }'
}

# Prints a rounded box. Arguments alternate plain text and its colored form.
summary_box() {
  local width=0 plain
  local -a plains=() painted=()
  while [ "$#" -gt 0 ]; do
    if [ "$#" -gt 2 ] || [ -n "$1" ]; then
      plains+=("$1")
      painted+=("$2")
    fi
    shift 2
  done
  for plain in "${plains[@]}"; do
    if [ "${#plain}" -gt "$width" ]; then
      width="${#plain}"
    fi
  done
  local i=0
  if [ $((width + 10)) -gt "$(terminal_columns)" ]; then
    printf '\n'
    while [ "$i" -lt "${#painted[@]}" ]; do
      printf '  %s\n' "${painted[$i]}"
      i=$((i + 1))
    done
    printf '\n'
    return
  fi
  local rule=""
  while [ "$i" -lt $((width + 6)) ]; do
    rule="${rule}${box_h}"
    i=$((i + 1))
  done
  local blank
  blank="$(printf '%*s' $((width + 6)) '')"
  printf '\n  %s\n' "${c_dim}${box_tl}${rule}${box_tr}${c_reset}"
  printf '  %s%s%s\n' "${c_dim}${box_v}${c_reset}" "$blank" "${c_dim}${box_v}${c_reset}"
  i=0
  while [ "$i" -lt "${#plains[@]}" ]; do
    local pad=$((width - ${#plains[$i]}))
    printf '  %s   %s%*s   %s\n' "${c_dim}${box_v}${c_reset}" "${painted[$i]}" "$pad" '' "${c_dim}${box_v}${c_reset}"
    i=$((i + 1))
  done
  printf '  %s%s%s\n' "${c_dim}${box_v}${c_reset}" "$blank" "${c_dim}${box_v}${c_reset}"
  printf '  %s\n\n' "${c_dim}${box_bl}${rule}${box_br}${c_reset}"
}

if ! command -v curl >/dev/null 2>&1; then
  die "curl is required"
fi

download() {
  url="$1"
  dest="$2"
  curl -fsSL --retry 3 --retry-delay 2 --proto '=https' --tlsv1.2 -o "$dest" "$url"
}

normalize_version() {
  raw="$1"
  case "$raw" in
    "" ) die "empty version" ;;
    *[!A-Za-z0-9._-]*) die "invalid version: ${raw}" ;;
  esac
  case "$raw" in
    v*) printf '%s\n' "$raw" ;;
    *) printf 'v%s\n' "$raw" ;;
  esac
}

resolve_version() {
  if [ -n "${1:-}" ]; then
    normalize_version "$1"
    return
  fi
  if [ -n "${CHATKEEP_VERSION:-}" ]; then
    normalize_version "$CHATKEEP_VERSION"
    return
  fi
  latest_url="${github}/releases/latest"
  effective=""
  if ! effective="$(curl -fsSL --proto '=https' --tlsv1.2 -o /dev/null -w '%{url_effective}' "$latest_url")"; then
    die "could not find the latest release at ${latest_url}"
  fi
  case "$effective" in
    */releases/tag/*) ;;
    *) die "unexpected latest-release URL: ${effective}" ;;
  esac
  tag="${effective##*/}"
  normalize_version "$tag"
}

target_triple() {
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Darwin)
      case "$arch" in
        x86_64) printf '%s\n' "x86_64-apple-darwin" ;;
        arm64 | aarch64) printf '%s\n' "aarch64-apple-darwin" ;;
        *) die "unsupported platform: ${os} ${arch}" ;;
      esac
      ;;
    Linux)
      if [ -f /etc/alpine-release ]; then
        die "unsupported platform: musl libc (Alpine). Published Linux builds require glibc."
      fi
      case "$arch" in
        x86_64 | amd64) printf '%s\n' "x86_64-unknown-linux-gnu" ;;
        arm64 | aarch64) printf '%s\n' "aarch64-unknown-linux-gnu" ;;
        *) die "unsupported platform: ${os} ${arch}" ;;
      esac
      ;;
    *)
      die "unsupported platform: ${os} ${arch}. Use install.ps1 on Windows."
      ;;
  esac
}

sha256_file() {
  file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{ print $1 }'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{ print $1 }'
  else
    die "sha256sum or shasum is required to verify the download"
  fi
}

verify_checksum() {
  file="$1"
  asset="$2"
  sums="$3"
  expected=""
  expected="$(grep -E "[[:space:]]\\*?${asset}$" "$sums" | awk 'NR == 1 { print $1 }' || true)"
  if [ -z "$expected" ]; then
    die "checksum mismatch: SHA256SUMS has no entry for ${asset}"
  fi
  expected="$(printf '%s' "$expected" | tr '[:upper:]' '[:lower:]')"
  actual="$(sha256_file "$file" | tr '[:upper:]' '[:lower:]')"
  if [ "$actual" != "$expected" ]; then
    error "checksum mismatch for ${asset}"
    error "expected ${expected}"
    error "actual   ${actual}"
    exit 1
  fi
}

install_dir="${CHATKEEP_INSTALL:-"${HOME}/.chatkeep/bin"}"
install_dir="${install_dir%/}"
if ! mkdir -p "$install_dir"; then
  die "could not create ${install_dir}"
fi
install_dir="$(cd "$install_dir" && pwd)"

print_logo

version="$(resolve_version "${1:-}")"
target="$(target_triple)"
asset="chatkeep-${target}.tar.gz"
base="${github}/releases/download/${version}"

step "Version" "${version}"
step "Platform" "${target}"

tmpdir=""
cleanup() {
  show_cursor
  if [ -n "$tmpdir" ]; then
    rm -rf "$tmpdir"
  fi
}
trap cleanup EXIT
tmpdir="$(mktemp -d)"

archive="${tmpdir}/${asset}"
sums="${tmpdir}/SHA256SUMS"

fetch_release() {
  if ! download "${base}/${asset}" "$archive"; then
    return 1
  fi
  download "${base}/SHA256SUMS" "$sums"
}

if ! spin "Downloading ${asset}" fetch_release; then
  if [ ! -s "$archive" ]; then
    die "download failed: ${base}/${asset}"
  fi
  die "download failed: ${base}/SHA256SUMS"
fi
step "Download" "${asset} ($(human_size "$(wc -c <"$archive" | tr -d ' ')"))"

verify_checksum "$archive" "$asset" "$sums"
step "Checksum" "SHA-256 matches SHA256SUMS"

extract="${tmpdir}/extract"
mkdir -p "$extract"
tar -xzf "$archive" -C "$extract"
src="${extract}/chatkeep"
if [ ! -f "$src" ]; then
  src="$(find "$extract" -type f -name chatkeep -print | head -n 1 || true)"
fi
if [ -z "$src" ] || [ ! -f "$src" ]; then
  die "archive does not contain chatkeep"
fi

dest="${install_dir}/chatkeep"
stage="${install_dir}/chatkeep.new"
cp "$src" "$stage"
chmod 755 "$stage"
mv -f "$stage" "$dest"
step "Installed" "$(display_path "$dest")"

rc=""
shell_name=""
case "${SHELL:-}" in
  */zsh)
    shell_name="zsh"
    rc="${HOME}/.zshrc"
    ;;
  */bash)
    shell_name="bash"
    if [ "$(uname -s)" = "Darwin" ]; then
      rc="${HOME}/.bash_profile"
    else
      rc="${HOME}/.bashrc"
    fi
    ;;
  */fish)
    shell_name="fish"
    rc="${HOME}/.config/fish/config.fish"
    ;;
  *)
    shell_name=""
    rc=""
    ;;
esac

path_recorded=0
for candidate in \
  "${HOME}/.zshrc" \
  "${HOME}/.bashrc" \
  "${HOME}/.bash_profile" \
  "${HOME}/.config/fish/config.fish"; do
  if [ -f "$candidate" ] && grep -F -q "$install_dir" "$candidate"; then
    path_recorded=1
  fi
done

rc_edited=0
if [ "$path_recorded" -eq 0 ] && [ -n "$rc" ]; then
  mkdir -p "$(dirname "$rc")"
  {
    printf '\n'
    printf '# chatkeep\n'
    if [ "$shell_name" = "fish" ]; then
      printf 'fish_add_path --prepend "%s"\n' "$install_dir"
    else
      literal_path_ref="\$PATH"
      printf 'export PATH="%s:%s"\n' "$install_dir" "$literal_path_ref"
    fi
  } >>"$rc"
  step "PATH" "added to $(display_path "$rc")"
  rc_edited=1
elif [ "$path_recorded" -eq 1 ]; then
  step "PATH" "already includes $(display_path "$install_dir")"
else
  note "PATH" "could not detect your shell rc file"
fi

# A child process cannot change the PATH of the shell that ran it, so link the binary into a
# directory that is already on PATH. Then chatkeep works at once, here and in every open terminal.
link=""
ready=0
case ":${PATH}:" in
  *":${install_dir}:"*) ready=1 ;;
esac
if [ "$ready" -eq 0 ]; then
  for link_dir in "${HOME}/.local/bin" "${HOME}/bin" /opt/homebrew/bin /usr/local/bin; do
    case ":${PATH}:" in
      *":${link_dir}:"*) ;;
      *) continue ;;
    esac
    if [ ! -d "$link_dir" ] || [ ! -w "$link_dir" ]; then
      continue
    fi
    candidate="${link_dir}/chatkeep"
    if [ -L "$candidate" ]; then
      # Replace only our own link or a dangling one.
      if [ "$(readlink "$candidate")" != "$dest" ] && [ -e "$candidate" ]; then
        continue
      fi
    elif [ -e "$candidate" ]; then
      continue
    fi
    if ln -sfn "$dest" "$candidate" 2>/dev/null; then
      link="$candidate"
      ready=1
      step "Linked" "$(display_path "$link")"
    fi
    break
  done
fi

run_cmd="${c_bold}${c_accent}chatkeep${c_reset}"
other_plain=""
other_painted=""
if [ "$ready" -eq 1 ]; then
  next_plain="run chatkeep (works in every open terminal)"
  next_painted="run ${run_cmd} ${c_dim}(works in every open terminal)${c_reset}"
elif [ "$rc_edited" -eq 1 ] || [ "$path_recorded" -eq 1 ]; then
  load="source $(display_path "${rc:-${HOME}/.zshrc}")"
  next_plain="${load} && chatkeep"
  next_painted="${c_bold}${c_accent}${load} && chatkeep${c_reset}"
  other_plain="Other     run ${load} in terminals that were already open"
  other_painted="${c_dim}Other${c_reset}     run ${c_accent}${load}${c_reset} in terminals that were already open"
else
  # ~ does not expand inside double quotes, so spell the home directory as $HOME.
  load="export PATH=\"${install_dir/#${HOME}/\$HOME}:\$PATH\""
  next_plain="${load} && chatkeep"
  next_painted="${c_bold}${c_accent}${load} && chatkeep${c_reset}"
fi

installed="${version#v}"
printed=""
if printed="$("$dest" --version 2>/dev/null)"; then
  installed="${printed##* }"
else
  note "Check" "could not run $(display_path "$dest") --version"
fi

title="chatkeep ${installed} is installed"
location="Location  $(display_path "$dest")"
summary_box \
  "${mark_ok} ${title}" "${c_bold}${c_green}${mark_ok}${c_reset} ${c_bold}${title}${c_reset}" \
  "" "" \
  "$location" "${c_dim}Location${c_reset}  $(display_path "$dest")" \
  "Next      ${next_plain}" "${c_dim}Next${c_reset}      ${next_painted}" \
  "$other_plain" "$other_painted"

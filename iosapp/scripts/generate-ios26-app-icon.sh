#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
design_dir="$repo_root/docs/designs/ios26-app-icon"
asset_dir="$repo_root/iosapp/XBXRC/Resources/Assets.xcassets/AppIcon.appiconset"
source_svg="${1:-$design_dir/icon-source.svg}"
temporary_dir="$(mktemp -d /tmp/xbxrc-ios26-app-icon.XXXXXX)"
symbol_canvas_size=1240
symbol_crop_offset=108

command -v rsvg-convert >/dev/null
command -v ffmpeg >/dev/null

rsvg-convert \
    --width "$symbol_canvas_size" \
    --height "$symbol_canvas_size" \
    --output "$temporary_dir/symbol-large.png" \
    "$source_svg"

ffmpeg -y -loglevel error \
    -i "$temporary_dir/symbol-large.png" \
    -vf "crop=1024:1024:$symbol_crop_offset:$symbol_crop_offset" \
    -frames:v 1 \
    -update 1 \
    "$temporary_dir/symbol.png"

render_icon() {
    local background_name="$1"
    local output_name="$2"
    local foreground_filter="$3"

    rsvg-convert \
        --width 1024 \
        --height 1024 \
        --output "$temporary_dir/background.png" \
        "$design_dir/$background_name"

    ffmpeg -y -loglevel error \
        -i "$temporary_dir/background.png" \
        -i "$temporary_dir/symbol.png" \
        -filter_complex "[1:v]$foreground_filter,split=2[shadow][symbol];[shadow]colorchannelmixer=rr=0:gg=0:bb=0:aa=0.30,boxblur=18:8[soft-shadow];[0:v][soft-shadow]overlay=0:22[with-shadow];[with-shadow][symbol]overlay=0:0,format=rgb24" \
        -frames:v 1 \
        -update 1 \
        "$asset_dir/$output_name"
}

render_icon \
    "background-default.svg" \
    "xbxrc-iOS-Default-1024x1024@1x.png" \
    "format=rgba"

render_icon \
    "background-dark.svg" \
    "xbxrc-iOS-Default-1024x1024@1x 1.png" \
    "format=rgba"

render_icon \
    "background-tinted.svg" \
    "xbxrc-iOS-Default-1024x1024@1x 2.png" \
    "format=rgba,lutrgb=r=255:g=255:b=255"

echo "Generated iOS 26 App Icon assets in $asset_dir"

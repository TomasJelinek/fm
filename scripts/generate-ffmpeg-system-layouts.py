#!/usr/bin/env python3
"""Generate, or check, the per-major FFmpeg ABI tables of the Linux system decoder.

The Linux platform helpers load the distribution's FFmpeg at run time (libavcodec 58 to 63)
and read a few public struct fields. Field offsets differ between major versions, so they
come from each release's own public headers: this script compiles a probe against them with
the host C compiler and writes plugins/midnight-thumbnail-video/src/system/linux/ffmpeg/
layouts.rs. A field is emitted for a major only when every probed release of that major
declares it at the same offset (the first and the last release of each major are probed),
so a field added within a major is never used.

    generate-ffmpeg-system-layouts.py HEADERS_ROOT --output FILE   write the Rust tables
    generate-ffmpeg-system-layouts.py HEADERS_ROOT --check FILE    fail if FILE differs

HEADERS_ROOT holds one folder per FFmpeg release, named by its version (4.0, 4.4.8, ...),
with that release's libavutil, libavcodec, libavformat, libswscale and libswresample
headers and a generated libavutil/avconfig.h (little-endian, fast unaligned). Fetch the
signed release archives from https://ffmpeg.org/releases/ and extract only the headers.
Nothing is downloaded or installed by this script; it needs a C compiler (cc).
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

# Releases probed per major: the first and the last of each.
RELEASES = {
    58: ("4.0", "4.4.8"),
    59: ("5.0", "5.1.10"),
    60: ("6.0", "6.1.6"),
    61: ("7.0", "7.1.5"),
    62: ("8.0", "8.1.3"),
    63: ("9.0", "9.1"),
}

# (Rust name, C type, member) for every field the decoder may touch.
FIELDS = [
    ("format_pb", "AVFormatContext", "pb"),
    ("format_nb_streams", "AVFormatContext", "nb_streams"),
    ("format_streams", "AVFormatContext", "streams"),
    ("format_start_time", "AVFormatContext", "start_time"),
    ("format_duration", "AVFormatContext", "duration"),
    ("format_io_open", "AVFormatContext", "io_open"),
    ("stream_index", "AVStream", "index"),
    ("stream_time_base", "AVStream", "time_base"),
    ("stream_codecpar", "AVStream", "codecpar"),
    ("stream_disposition", "AVStream", "disposition"),
    ("par_codec_type", "AVCodecParameters", "codec_type"),
    ("par_codec_id", "AVCodecParameters", "codec_id"),
    ("par_width", "AVCodecParameters", "width"),
    ("par_height", "AVCodecParameters", "height"),
    ("par_sample_rate", "AVCodecParameters", "sample_rate"),
    ("par_coded_side_data", "AVCodecParameters", "coded_side_data"),
    ("par_nb_coded_side_data", "AVCodecParameters", "nb_coded_side_data"),
    ("codec_thread_count", "AVCodecContext", "thread_count"),
    ("codec_thread_type", "AVCodecContext", "thread_type"),
    ("codec_max_pixels", "AVCodecContext", "max_pixels"),
    ("codec_pkt_timebase", "AVCodecContext", "pkt_timebase"),
    ("frame_data", "AVFrame", "data"),
    ("frame_linesize", "AVFrame", "linesize"),
    ("frame_extended_data", "AVFrame", "extended_data"),
    ("frame_width", "AVFrame", "width"),
    ("frame_height", "AVFrame", "height"),
    ("frame_nb_samples", "AVFrame", "nb_samples"),
    ("frame_format", "AVFrame", "format"),
    ("frame_pts", "AVFrame", "pts"),
    ("frame_best_effort_timestamp", "AVFrame", "best_effort_timestamp"),
    ("frame_sample_rate", "AVFrame", "sample_rate"),
    ("frame_channels", "AVFrame", "channels"),
    ("frame_channel_layout", "AVFrame", "channel_layout"),
    ("frame_ch_layout", "AVFrame", "ch_layout"),
    ("frame_color_range", "AVFrame", "color_range"),
    ("frame_color_primaries", "AVFrame", "color_primaries"),
    ("frame_color_trc", "AVFrame", "color_trc"),
    ("frame_colorspace", "AVFrame", "colorspace"),
    ("packet_pts", "AVPacket", "pts"),
    ("packet_stream_index", "AVPacket", "stream_index"),
    ("io_buffer", "AVIOContext", "buffer"),
    ("side_data_data", "AVPacketSideData", "data"),
    ("side_data_size", "AVPacketSideData", "size"),
    ("side_data_type", "AVPacketSideData", "type"),
    ("ch_layout_order", "AVChannelLayout", "order"),
    ("ch_layout_nb_channels", "AVChannelLayout", "nb_channels"),
    ("ch_layout_mask", "AVChannelLayout", "u.mask"),
]

# (Rust name, C type) sizes.
SIZES = [
    ("side_data_size_of", "AVPacketSideData"),
    ("side_data_size_type_size", "((AVPacketSideData *)0)->size"),
    ("ch_layout_size_of", "AVChannelLayout"),
]

# Exact constants the decoder relies on; each must be equal in every probed release.
CONSTANTS = [
    "AVERROR_EOF", "AVERROR(EAGAIN)", "AVSEEK_SIZE", "AVSEEK_FORCE", "AVFMT_FLAG_CUSTOM_IO",
    "AV_DISPOSITION_ATTACHED_PIC", "AVMEDIA_TYPE_VIDEO", "AVMEDIA_TYPE_AUDIO",
    "AV_PKT_DATA_DISPLAYMATRIX", "AV_PIX_FMT_RGBA", "AV_SAMPLE_FMT_S16", "SWS_BILINEAR",
    "SWS_CS_ITU709", "SWS_CS_ITU601", "AV_CHANNEL_ORDER_NATIVE", "AV_CH_LAYOUT_STEREO",
]

# Library majors of each release, from its version headers.
VERSIONS = [
    ("avcodec", "LIBAVCODEC_VERSION_INT"),
    ("avformat", "LIBAVFORMAT_VERSION_INT"),
    ("avutil", "LIBAVUTIL_VERSION_INT"),
    ("swscale", "LIBSWSCALE_VERSION_INT"),
    ("swresample", "LIBSWRESAMPLE_VERSION_INT"),
]

# Every function the decoder resolves. Each one's preprocessed prototype is recorded per
# major (the reviewed ABI surface); a function is present for a major only when every
# probed release declares it. The decoder's function-pointer types follow these.
FUNCTIONS = [
    "avutil_version", "av_malloc", "av_free", "av_dict_set", "av_dict_free", "av_log_set_level",
    "av_frame_alloc", "av_frame_free", "av_frame_unref", "av_frame_move_ref",
    "av_display_rotation_get",
    "av_get_default_channel_layout", "av_channel_layout_default", "av_channel_layout_copy",
    "av_channel_layout_uninit",
    "avcodec_version", "avcodec_find_decoder", "avcodec_alloc_context3",
    "avcodec_parameters_to_context", "avcodec_open2", "avcodec_send_packet",
    "avcodec_receive_frame", "avcodec_flush_buffers", "avcodec_free_context", "avcodec_get_name",
    "av_packet_alloc", "av_packet_free", "av_packet_unref", "av_packet_side_data_get",
    "avformat_version", "avformat_alloc_context", "avformat_free_context", "avformat_open_input",
    "avformat_find_stream_info", "avformat_close_input", "av_find_best_stream", "av_read_frame",
    "avformat_seek_file", "avio_alloc_context", "avio_context_free", "av_stream_get_side_data",
    "swscale_version", "sws_getContext", "sws_scale", "sws_freeContext",
    "sws_setColorspaceDetails", "sws_getCoefficients",
    "swresample_version", "swr_alloc_set_opts", "swr_alloc_set_opts2", "swr_init", "swr_convert",
    "swr_get_out_samples", "swr_free",
]

# Presence flags emitted into the tables: the decoder picks the API generation from them.
PRESENCE = [
    "av_stream_get_side_data", "av_packet_side_data_get", "swr_alloc_set_opts",
    "swr_alloc_set_opts2", "av_get_default_channel_layout", "av_channel_layout_default",
]

INCLUDES = """#include <stddef.h>
#include <stdio.h>
#include <errno.h>
#include <libavutil/avutil.h>
#include <libavutil/frame.h>
#include <libavutil/channel_layout.h>
#include <libavutil/display.h>
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavformat/avio.h>
#include <libswscale/swscale.h>
#include <libswresample/swresample.h>
"""

HEADER = INCLUDES + "int main(void) {\n"


def probe_source(fields, sizes, constants):
    lines = [HEADER]
    for name, kind, member in fields:
        lines.append(f'    printf("field {name} %zu\\n", offsetof({kind}, {member}));\n')
    for name, expression in sizes:
        lines.append(f'    printf("size {name} %zu\\n", sizeof({expression}));\n')
    for constant in constants:
        lines.append(f'    printf("constant {constant} %lld\\n", (long long)({constant}));\n')
    for library, macro in VERSIONS:
        lines.append(f'    printf("version {library} %d\\n", (int)({macro}));\n')
    lines.append("    return 0;\n}\n")
    return "".join(lines)


MISSING = [
    # 'AVFrame' {aka 'struct AVFrame'} has no member named 'ch_layout'
    (re.compile(r"'(\w+)'(?: \{aka '[^']*'\})? has no member named '(\w+)'"), "member"),
    (re.compile(r"'(\w+)' undeclared"), "identifier"),
    (re.compile(r"unknown type name '(\w+)'"), "type"),
    (re.compile(r"invalid application of 'sizeof' to incomplete type '(?:struct )?(\w+)'"), "type"),
]


def probe(release_dir, compiler):
    fields, sizes, constants = list(FIELDS), list(SIZES), list(CONSTANTS)
    with tempfile.TemporaryDirectory(prefix="ffmpeg-layout-") as work:
        source, binary = Path(work, "probe.c"), Path(work, "probe")
        for _ in range(64):
            source.write_text(probe_source(fields, sizes, constants), encoding="utf-8")
            result = subprocess.run(
                [compiler, "-std=gnu11", "-w", "-I", str(release_dir), str(source), "-o", str(binary)],
                capture_output=True, text=True, env={**os.environ, "LC_ALL": "C"})
            if result.returncode == 0:
                break
            members, identifiers = set(), set()
            for pattern, kind in MISSING:
                for match in pattern.finditer(result.stderr):
                    if kind == "member":
                        members.add((match.group(1), match.group(2)))
                    else:
                        identifiers.add(match.group(1))
            if not members and not identifiers:
                raise SystemExit(f"Probe failed for {release_dir}:\n{result.stderr}")
            fields = [field for field in fields
                      if (field[1], field[2].split(".")[0]) not in members and field[1] not in identifiers]
            sizes = [size for size in sizes
                     if not any(identifier in size[1] for identifier in identifiers)]
            constants = [constant for constant in constants
                         if not any(re.search(rf"\b{identifier}\b", constant) for identifier in identifiers)]
        else:
            raise SystemExit(f"Probe did not converge for {release_dir}")
        output = subprocess.run([str(binary)], capture_output=True, text=True, check=True).stdout
    values = {"field": {}, "size": {}, "constant": {}, "version": {}}
    for line in output.splitlines():
        kind, name, value = line.split(" ")
        values[kind][name] = int(value)
    values["prototype"] = prototypes(release_dir, compiler)
    return values


def prototypes(release_dir, compiler):
    """Each function's declaration after preprocessing: conditionals such as the
    FF_API_* switches are resolved exactly as for a program built against the release."""
    with tempfile.TemporaryDirectory(prefix="ffmpeg-decls-") as work:
        source = Path(work, "decls.c")
        source.write_text(INCLUDES, encoding="utf-8")
        result = subprocess.run(
            [compiler, "-E", "-P", "-w", "-I", str(release_dir), str(source)],
            capture_output=True, text=True, check=True, env={**os.environ, "LC_ALL": "C"})
    text = " ".join(result.stdout.split())
    found = {}
    for name in FUNCTIONS:
        found[name] = None
        for match in re.finditer(rf"\b{name}\s*\(", text):
            start = max(text.rfind(";", 0, match.start()), text.rfind("}", 0, match.start()),
                        text.rfind("{", 0, match.start())) + 1
            end = text.find(";", match.end())
            declaration = text[start:end].strip()
            if "{" in declaration or declaration.startswith("return") or "=" in declaration:
                continue
            declaration = re.sub(r"__attribute__\s*\(\((?:[^()]|\([^()]*\))*\)\)\s*", "", declaration)
            found[name] = " ".join(declaration.split())
            break
    return found


def combine(root, compiler):
    majors = {}
    for major, releases in RELEASES.items():
        probes = {release: probe(Path(root, release), compiler) for release in releases}
        versions = [probes[release]["version"] for release in releases]
        for release, version in zip(releases, versions):
            if version["avcodec"] >> 16 != major:
                raise SystemExit(f"FFmpeg {release} is libavcodec {version['avcodec'] >> 16}, not {major}")
        for library in ("avformat", "avutil", "swscale", "swresample"):
            if len({version[library] >> 16 for version in versions}) != 1:
                raise SystemExit(f"libav{library} major changes within libavcodec {major}")
        fields = {}
        for name, _, _ in FIELDS:
            offsets = {probes[release]["field"].get(name) for release in releases}
            fields[name] = offsets.pop() if len(offsets) == 1 else None
        sizes = {}
        for name, _ in SIZES:
            values = {probes[release]["size"].get(name) for release in releases}
            sizes[name] = values.pop() if len(values) == 1 else None
        for constant in CONSTANTS:
            # A constant of an API added within the major is absent from the first release.
            values = {probes[release]["constant"].get(constant) for release in releases} - {None}
            if len(values) > 1:
                raise SystemExit(f"{constant} differs within libavcodec {major}: {values}")
        declarations = {}
        for function in FUNCTIONS:
            forms = {probes[release]["prototype"][function] for release in releases}
            declarations[function] = sorted(form or "absent" for form in forms)
        presence = {function: all(probes[release]["prototype"][function] for release in releases)
                    for function in PRESENCE}
        side_data = {probes[release]["prototype"]["av_stream_get_side_data"] for release in releases}
        int_size = None
        if presence["av_stream_get_side_data"]:
            kinds = {"int" if re.search(r"\bint\s*\*\s*size\s*\)$", form) else
                     "size_t" if re.search(r"\bsize_t\s*\*\s*size\s*\)$", form) else form
                     for form in side_data}
            if kinds == {"int"}:
                int_size = True
            elif kinds == {"size_t"}:
                int_size = False
            else:
                raise SystemExit(f"av_stream_get_side_data varies within libavcodec {major}: {kinds}")
        majors[major] = {
            "releases": releases,
            "first": versions[0],
            "libraries": {library: versions[0][library] >> 16 for library, _ in VERSIONS},
            "fields": fields,
            "sizes": sizes,
            "constants": {constant: probes[releases[-1]]["constant"].get(constant) for constant in CONSTANTS},
            "declarations": declarations,
            "presence": presence,
            "stream_side_data_int_size": int_size,
        }
    return majors


def render(majors):
    out = [
        "// @generated by scripts/generate-ffmpeg-system-layouts.py from the public headers of",
        "// the FFmpeg releases named below. Do not edit; regenerate, and run the script with",
        "// --check against the same headers to verify. Offsets are in bytes on 64-bit Linux.",
        "",
        "/// Struct field offsets of one FFmpeg major release series. `None`: the field is absent",
        "/// in at least one release of that series, or at different offsets, and is never used.",
        "#[derive(Debug)]",
        "pub(super) struct Layout {",
        "    /// libavcodec, libavformat, libavutil, libswscale and libswresample majors.",
        "    pub avcodec: u32,",
        "    pub avformat: u32,",
        "    pub avutil: u32,",
        "    pub swscale: u32,",
        "    pub swresample: u32,",
        "    /// The oldest release of the series as libavcodec's AV_VERSION_INT.",
        "    pub first_avcodec_version: u32,",
    ]
    for name, kind, member in FIELDS:
        out.append(f"    /// `{kind}::{member}`")
        out.append(f"    pub {name}: Option<usize>,")
    for name, expression in SIZES:
        out.append(f"    /// `sizeof({expression})`")
        out.append(f"    pub {name}: Option<usize>,")
    for function in PRESENCE:
        out.append(f"    /// `{function}` is declared by every release of the series.")
        out.append(f"    pub has_{function}: bool,")
    out.append("    /// `av_stream_get_side_data` takes `int *size` (true) or `size_t *size` (false).")
    out.append("    pub stream_side_data_int_size: Option<bool>,")
    out.append("}")
    out.append("")
    out.append("/// One entry per supported libavcodec major, oldest first.")
    out.append(f"pub(super) const LAYOUTS: [Layout; {len(majors)}] = [")
    for major, data in majors.items():
        out.append(f"    // FFmpeg {' and '.join(data['releases'])}")
        out.append("    Layout {")
        for library in ("avcodec", "avformat", "avutil", "swscale", "swresample"):
            out.append(f"        {library}: {data['libraries'][library]},")
        out.append(f"        first_avcodec_version: {data['first']['avcodec']:#x},")
        for name, _, _ in FIELDS:
            value = data["fields"][name]
            out.append(f"        {name}: {'None' if value is None else f'Some({value})'},")
        for name, _ in SIZES:
            value = data["sizes"][name]
            out.append(f"        {name}: {'None' if value is None else f'Some({value})'},")
        for function in PRESENCE:
            out.append(f"        has_{function}: {'true' if data['presence'][function] else 'false'},")
        flag = data["stream_side_data_int_size"]
        out.append(f"        stream_side_data_int_size: {'None' if flag is None else f'Some({str(flag).lower()})'},")
        out.append("    },")
    out.append("];")
    out.append("")
    out.append("// Constants checked equal within each series (one value per series, oldest first;")
    out.append("// None: absent from at least one release):")
    for constant in CONSTANTS:
        values = [str(data["constants"].get(constant)) for data in majors.values()]
        out.append(f"// {constant} = {', '.join(values)}")
    out.append("//")
    out.append("// Declarations after preprocessing, per series; two forms when they differ within it.")
    for major, data in majors.items():
        out.append(f"//")
        out.append(f"// libavcodec {major} (FFmpeg {' and '.join(data['releases'])}):")
        for function, forms in data["declarations"].items():
            for form in forms:
                out.append(f"//   {form if form != 'absent' else function + ': absent'}")
    return "\n".join(out) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("headers_root", type=Path)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--output", type=Path)
    group.add_argument("--check", type=Path)
    parser.add_argument("--compiler", default=os.environ.get("CC", "cc"))
    arguments = parser.parse_args()
    text = render(combine(arguments.headers_root, arguments.compiler))
    if arguments.output:
        arguments.output.write_text(text, encoding="utf-8", newline="\n")
        return 0
    current = arguments.check.read_text(encoding="utf-8")
    if current != text:
        sys.stderr.write(f"{arguments.check} does not match the headers in {arguments.headers_root}\n")
        return 1
    print(f"{arguments.check} matches {sum(len(r) for r in RELEASES.values())} FFmpeg releases")
    return 0


if __name__ == "__main__":
    sys.exit(main())

// FFmpeg declarations used by the Rust media owners
#pragma once

#include <errno.h>
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/display.h>
#include <libavutil/hwcontext.h>
#include <libavutil/hwcontext_vulkan.h>
#include <libavutil/pixdesc.h>
#include <libswresample/swresample.h>
#include <libswscale/swscale.h>

static const int FLOWA_AVERROR_AGAIN = AVERROR(EAGAIN);
static const int FLOWA_AVERROR_EOF = AVERROR_EOF;
static const int FLOWA_AVERROR_EXIT = AVERROR_EXIT;

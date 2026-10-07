// Builds AMD's MSVC-only code with GCC and Clang (force-included by CMakeLists.txt).
#pragma once
#ifndef _WIN32
#include <cstring>
#include <cstdio>
#include <cmath>
#include <cwchar>
#include <cstdarg>
#include <cstddef>
#include <cerrno>
#include <new>
#include <locale>
#include <codecvt>

#define FFX_UNUSED(x)               ((void)(x))

template <typename T, size_t N>
constexpr size_t _countof_impl(T (&)[N]) { return N; }
#define _countof(a) _countof_impl(a)

inline int wcscpy_s(wchar_t* dst, size_t n, const wchar_t* src)
{
    if (!dst || !src || n == 0)
        return EINVAL;
    size_t len = wcslen(src);
    if (len >= n)
    {
        dst[0] = 0;
        return ERANGE;
    }
    wmemcpy(dst, src, len + 1);
    return 0;
}
template <size_t N>
inline int wcscpy_s(wchar_t (&dst)[N], const wchar_t* src) { return wcscpy_s(dst, N, src); }

inline int strcpy_s(char* dst, size_t n, const char* src)
{
    if (!dst || !src || n == 0)
        return EINVAL;
    size_t len = strlen(src);
    if (len >= n)
    {
        dst[0] = 0;
        return ERANGE;
    }
    memcpy(dst, src, len + 1);
    return 0;
}
template <size_t N>
inline int strcpy_s(char (&dst)[N], const char* src) { return strcpy_s(dst, N, src); }

inline int sprintf_s(char* dst, size_t n, const char* fmt, ...)
{
    va_list args;
    va_start(args, fmt);
    int r = vsnprintf(dst, n, fmt, args);
    va_end(args);
    return r;
}
template <size_t N>
inline int sprintf_s(char (&dst)[N], const char* fmt, ...)
{
    va_list args;
    va_start(args, fmt);
    int r = vsnprintf(dst, N, fmt, args);
    va_end(args);
    return r;
}
#endif

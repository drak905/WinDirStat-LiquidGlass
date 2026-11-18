// WinDirStat - Windows Directory Statistics
// Copyright © WinDirStat Team
//
// SPDX-License-Identifier: GPL-3.0-or-later
// Distributed WITHOUT ANY WARRANTY; see LICENSE.md for details.

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>

#include "IpcStreamer.h"

#include <algorithm>
#include <array>
#include <cstdio>
#include <cwctype>
#include <utility>

namespace wds::ipc
{
    namespace
    {
        std::wstring_view TrimView(std::wstring_view sv) noexcept
        {
            while (!sv.empty() && std::iswspace(sv.front())) sv.remove_prefix(1);
            while (!sv.empty() && std::iswspace(sv.back())) sv.remove_suffix(1);
            return sv;
        }

        bool StartsWithCaseInsensitive(std::wstring_view haystack, std::wstring_view needle) noexcept
        {
            if (haystack.size() < needle.size()) return false;
            for (std::size_t i = 0; i < needle.size(); ++i)
            {
                if (std::towlower(haystack[i]) != std::towlower(needle[i])) return false;
            }
            return true;
        }

        bool EqualsCaseInsensitive(std::wstring_view a, std::wstring_view b) noexcept
        {
            return a.size() == b.size() && StartsWithCaseInsensitive(a, b);
        }

        std::string_view FrameTypeToString(const IpcFrameType type) noexcept
        {
            switch (type)
            {
            case IpcFrameType::Nodes: return "nodes";
            case IpcFrameType::Complete: return "complete";
            case IpcFrameType::Error: return "error";
            }
            return "nodes";
        }

        void AppendJsonEscaped(std::string& out, std::string_view sv)
        {
            out.push_back('"');
            for (const unsigned char c : sv)
            {
                switch (c)
                {
                case '"': out += "\\\""; break;
                case '\\': out += "\\\\"; break;
                case '\b': out += "\\b"; break;
                case '\f': out += "\\f"; break;
                case '\n': out += "\\n"; break;
                case '\r': out += "\\r"; break;
                case '\t': out += "\\t"; break;
                default:
                    if (c < 0x20)
                    {
                        char buf[7];
                        std::snprintf(buf, sizeof(buf), "\\u%04x", static_cast<unsigned>(c));
                        out += buf;
                    }
                    else
                    {
                        out.push_back(static_cast<char>(c));
                    }
                    break;
                }
            }
            out.push_back('"');
        }

        void AppendNodeJson(std::string& out, const IpcNodeRecord& node)
        {
            out += "{\"id\":";
            out += std::to_string(node.id);
            out += ",\"parent_id\":";
            if (node.parent_id.has_value()) out += std::to_string(*node.parent_id);
            else out += "null";
            out += ",\"name\":";
            AppendJsonEscaped(out, node.name);
            out += ",\"full_path\":";
            AppendJsonEscaped(out, node.full_path);
            out += ",\"is_dir\":";
            out += (node.is_dir ? "true" : "false");
            out += ",\"logical_size\":";
            out += std::to_string(node.logical_size);
            out += ",\"physical_size\":";
            out += std::to_string(node.physical_size);
            out += ",\"modified_utc\":";
            AppendJsonEscaped(out, node.modified_utc);
            out += ",\"owner\":";
            AppendJsonEscaped(out, node.owner);
            out += ",\"xxhash64\":";
            if (node.xxhash64.has_value()) out += std::to_string(*node.xxhash64);
            else out += "null";
            out += ",\"extension\":";
            AppendJsonEscaped(out, node.extension);
            out.push_back('}');
        }

        // Minimal spec-compliant MessagePack encoder matching rmp-serde named map layout
        class MsgPackEncoder final
        {
        public:
            void WriteNil() { m_buf.push_back(0xC0); }
            void WriteBool(const bool v) { m_buf.push_back(v ? 0xC3 : 0xC2); }

            void WriteUInt(const std::uint64_t v)
            {
                if (v <= 0x7Fu)
                {
                    m_buf.push_back(static_cast<std::uint8_t>(v));
                }
                else if (v <= 0xFFu)
                {
                    m_buf.push_back(0xCC);
                    m_buf.push_back(static_cast<std::uint8_t>(v));
                }
                else if (v <= 0xFFFFu)
                {
                    m_buf.push_back(0xCD);
                    WriteBe16(static_cast<std::uint16_t>(v));
                }
                else if (v <= 0xFFFFFFFFu)
                {
                    m_buf.push_back(0xCE);
                    WriteBe32(static_cast<std::uint32_t>(v));
                }
                else
                {
                    m_buf.push_back(0xCF);
                    WriteBe64(v);
                }
            }

            void WriteStr(std::string_view s)
            {
                const std::size_t len = s.size();
                if (len <= 31)
                {
                    m_buf.push_back(static_cast<std::uint8_t>(0xA0 | len));
                }
                else if (len <= 0xFFu)
                {
                    m_buf.push_back(0xD9);
                    m_buf.push_back(static_cast<std::uint8_t>(len));
                }
                else if (len <= 0xFFFFu)
                {
                    m_buf.push_back(0xDA);
                    WriteBe16(static_cast<std::uint16_t>(len));
                }
                else
                {
                    m_buf.push_back(0xDB);
                    WriteBe32(static_cast<std::uint32_t>(len));
                }
                m_buf.insert(m_buf.end(),
                    reinterpret_cast<const std::uint8_t*>(s.data()),
                    reinterpret_cast<const std::uint8_t*>(s.data()) + len);
            }

            void WriteArrayHeader(const std::size_t count)
            {
                if (count <= 15)
                {
                    m_buf.push_back(static_cast<std::uint8_t>(0x90 | count));
                }
                else if (count <= 0xFFFFu)
                {
                    m_buf.push_back(0xDC);
                    WriteBe16(static_cast<std::uint16_t>(count));
                }
                else
                {
                    m_buf.push_back(0xDD);
                    WriteBe32(static_cast<std::uint32_t>(count));
                }
            }

            void WriteMapHeader(const std::size_t count)
            {
                if (count <= 15)
                {
                    m_buf.push_back(static_cast<std::uint8_t>(0x80 | count));
                }
                else if (count <= 0xFFFFu)
                {
                    m_buf.push_back(0xDE);
                    WriteBe16(static_cast<std::uint16_t>(count));
                }
                else
                {
                    m_buf.push_back(0xDF);
                    WriteBe32(static_cast<std::uint32_t>(count));
                }
            }

            void WriteKeyValue(std::string_view key, const std::uint64_t v)
            {
                WriteStr(key);
                WriteUInt(v);
            }

            void WriteKeyValue(std::string_view key, const bool v)
            {
                WriteStr(key);
                WriteBool(v);
            }

            void WriteKeyValue(std::string_view key, std::string_view v)
            {
                WriteStr(key);
                WriteStr(v);
            }

            void WriteKeyOptionalUInt(std::string_view key, const std::optional<std::uint64_t> v)
            {
                WriteStr(key);
                if (v.has_value()) WriteUInt(*v);
                else WriteNil();
            }

            void WriteKeyOptionalStr(std::string_view key, const std::optional<std::string>& v)
            {
                WriteStr(key);
                if (v.has_value()) WriteStr(*v);
                else WriteNil();
            }

            std::vector<std::uint8_t> Take() noexcept { return std::move(m_buf); }

        private:
            void WriteBe16(const std::uint16_t v)
            {
                m_buf.push_back(static_cast<std::uint8_t>((v >> 8) & 0xFF));
                m_buf.push_back(static_cast<std::uint8_t>(v & 0xFF));
            }

            void WriteBe32(const std::uint32_t v)
            {
                m_buf.push_back(static_cast<std::uint8_t>((v >> 24) & 0xFF));
                m_buf.push_back(static_cast<std::uint8_t>((v >> 16) & 0xFF));
                m_buf.push_back(static_cast<std::uint8_t>((v >> 8) & 0xFF));
                m_buf.push_back(static_cast<std::uint8_t>(v & 0xFF));
            }

            void WriteBe64(const std::uint64_t v)
            {
                for (int shift = 56; shift >= 0; shift -= 8)
                {
                    m_buf.push_back(static_cast<std::uint8_t>((v >> shift) & 0xFF));
                }
            }

            std::vector<std::uint8_t> m_buf;
        };
    } // namespace

    // ── Buffer writer ────────────────────────────────────────────────────────

    bool IpcBufferWriter::IsOpen() const noexcept
    {
        std::scoped_lock lock(m_mutex);
        return m_open;
    }

    bool IpcBufferWriter::WriteBytes(const std::span<const std::uint8_t> data)
    {
        std::scoped_lock lock(m_mutex);
        if (!m_open) return false;
        m_buffer.insert(m_buffer.end(), data.begin(), data.end());
        return true;
    }

    bool IpcBufferWriter::Flush()
    {
        std::scoped_lock lock(m_mutex);
        return m_open;
    }

    void IpcBufferWriter::Close() noexcept
    {
        std::scoped_lock lock(m_mutex);
        m_open = false;
    }

    std::string_view IpcBufferWriter::AsStringView() const noexcept
    {
        return std::string_view(reinterpret_cast<const char*>(m_buffer.data()), m_buffer.size());
    }

    void IpcBufferWriter::Clear() noexcept
    {
        std::scoped_lock lock(m_mutex);
        m_buffer.clear();
    }

    // ── Named pipe writer ────────────────────────────────────────────────────

    IpcNamedPipeWriter::IpcNamedPipeWriter(std::wstring pipePath, const std::uint32_t connectTimeoutMs)
        : m_pipePath(std::move(pipePath))
    {
        if (const auto normalized = NormalizePipePath(m_pipePath))
        {
            m_pipePath = *normalized;
            Connect(connectTimeoutMs);
        }
        else
        {
            m_lastError = ERROR_INVALID_NAME;
        }
    }

    IpcNamedPipeWriter::~IpcNamedPipeWriter()
    {
        Close();
    }

    IpcNamedPipeWriter::IpcNamedPipeWriter(IpcNamedPipeWriter&& other) noexcept
        : m_pipePath(std::move(other.m_pipePath))
        , m_handle(std::exchange(other.m_handle, nullptr))
        , m_lastError(std::exchange(other.m_lastError, 0))
    {
    }

    IpcNamedPipeWriter& IpcNamedPipeWriter::operator=(IpcNamedPipeWriter&& other) noexcept
    {
        if (this != &other)
        {
            Close();
            m_pipePath = std::move(other.m_pipePath);
            m_handle = std::exchange(other.m_handle, nullptr);
            m_lastError = std::exchange(other.m_lastError, 0);
        }
        return *this;
    }

    bool IpcNamedPipeWriter::Connect(const std::uint32_t connectTimeoutMs)
    {
        const auto deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(connectTimeoutMs);
        while (true)
        {
            HANDLE h = CreateFileW(
                m_pipePath.c_str(),
                GENERIC_WRITE,
                0,
                nullptr,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                nullptr);
            if (h != INVALID_HANDLE_VALUE)
            {
                m_handle = h;
                m_lastError = ERROR_SUCCESS;
                return true;
            }

            m_lastError = GetLastError();
            const auto now = std::chrono::steady_clock::now();
            if (now >= deadline) return false;

            const auto remainingMs = static_cast<DWORD>(
                std::chrono::duration_cast<std::chrono::milliseconds>(deadline - now).count());
            if (m_lastError == ERROR_PIPE_BUSY)
            {
                WaitNamedPipeW(m_pipePath.c_str(), std::max<DWORD>(1, remainingMs));
            }
            else if (m_lastError == ERROR_FILE_NOT_FOUND)
            {
                Sleep(std::min<DWORD>(20, std::max<DWORD>(1, remainingMs)));
            }
            else
            {
                return false;
            }
        }
    }

    bool IpcNamedPipeWriter::IsOpen() const noexcept
    {
        return m_handle != nullptr && m_handle != INVALID_HANDLE_VALUE;
    }

    bool IpcNamedPipeWriter::WriteBytes(const std::span<const std::uint8_t> data)
    {
        if (!IsOpen()) return false;
        std::size_t offset = 0;
        while (offset < data.size())
        {
            const DWORD chunk = static_cast<DWORD>(
                std::min<std::size_t>(data.size() - offset, 64u * 1024u));
            DWORD written = 0;
            if (!WriteFile(static_cast<HANDLE>(m_handle), data.data() + offset, chunk, &written, nullptr))
            {
                m_lastError = GetLastError();
                return false;
            }
            if (written == 0)
            {
                m_lastError = ERROR_WRITE_FAULT;
                return false;
            }
            offset += written;
        }
        return true;
    }

    bool IpcNamedPipeWriter::Flush()
    {
        if (!IsOpen()) return false;
        if (!FlushFileBuffers(static_cast<HANDLE>(m_handle)))
        {
            m_lastError = GetLastError();
            return false;
        }
        return true;
    }

    void IpcNamedPipeWriter::Close() noexcept
    {
        if (IsOpen())
        {
            CloseHandle(static_cast<HANDLE>(m_handle));
        }
        m_handle = nullptr;
    }

    // ── Progressive streamer ─────────────────────────────────────────────────

    IpcStreamer::IpcStreamer(std::shared_ptr<IIpcWriter> writer, IpcStreamerOptions options)
        : m_writer(std::move(writer))
        , m_options(std::move(options))
        , m_lastFlush(std::chrono::steady_clock::now())
    {
        if (m_options.batch_size == 0) m_options.batch_size = 1;
        m_pendingNodes.reserve(m_options.batch_size);
    }

    bool IpcStreamer::PushNode(IpcNodeRecord node)
    {
        std::scoped_lock lock(m_mutex);
        if (m_finished || !m_writer || !m_writer->IsOpen()) return false;

        ++m_scannedNodes;
        if (!node.is_dir) m_totalBytes += node.logical_size;
        m_pendingNodes.push_back(std::move(node));

        const bool batchFull = m_pendingNodes.size() >= m_options.batch_size;
        bool intervalElapsed = false;
        if (!batchFull && m_options.flush_interval_ms > 0)
        {
            const auto elapsed = std::chrono::duration_cast<std::chrono::milliseconds>(
                std::chrono::steady_clock::now() - m_lastFlush);
            intervalElapsed = elapsed.count() >= static_cast<long long>(m_options.flush_interval_ms);
        }

        if (batchFull || intervalElapsed)
        {
            return FlushBatchLocked();
        }
        return true;
    }

    bool IpcStreamer::FlushBatch()
    {
        std::scoped_lock lock(m_mutex);
        return FlushBatchLocked();
    }

    bool IpcStreamer::Finish(const std::string_view errorMessage)
    {
        std::scoped_lock lock(m_mutex);
        if (m_finished) return true;
        if (!m_writer || !m_writer->IsOpen()) return false;

        if (!m_pendingNodes.empty() && !FlushBatchLocked())
        {
            return false;
        }

        IpcStreamFrame finalFrame;
        finalFrame.frame_type = errorMessage.empty() ? IpcFrameType::Complete : IpcFrameType::Error;
        finalFrame.seq = ++m_seq;
        finalFrame.scan_root = m_options.scan_root;
        finalFrame.scanned_nodes = m_scannedNodes;
        finalFrame.total_bytes = m_totalBytes;
        finalFrame.is_final = true;
        if (!errorMessage.empty()) finalFrame.error = std::string(errorMessage);

        if (!WriteFrameLocked(finalFrame)) return false;
        m_finished = true;
        return m_writer->Flush();
    }

    std::uint64_t IpcStreamer::ScannedNodes() const noexcept
    {
        std::scoped_lock lock(m_mutex);
        return m_scannedNodes;
    }

    std::uint64_t IpcStreamer::TotalBytes() const noexcept
    {
        std::scoped_lock lock(m_mutex);
        return m_totalBytes;
    }

    std::uint64_t IpcStreamer::FramesSent() const noexcept
    {
        std::scoped_lock lock(m_mutex);
        return m_seq;
    }

    bool IpcStreamer::FlushBatchLocked()
    {
        if (m_pendingNodes.empty()) return true;
        if (!m_writer || !m_writer->IsOpen()) return false;

        IpcStreamFrame frame;
        frame.frame_type = IpcFrameType::Nodes;
        frame.seq = ++m_seq;
        frame.scan_root = m_options.scan_root;
        frame.scanned_nodes = m_scannedNodes;
        frame.total_bytes = m_totalBytes;
        frame.is_final = false;
        frame.nodes = std::move(m_pendingNodes);
        m_pendingNodes.clear();
        m_pendingNodes.reserve(m_options.batch_size);

        const bool ok = WriteFrameLocked(frame);
        m_lastFlush = std::chrono::steady_clock::now();
        return ok;
    }

    bool IpcStreamer::WriteFrameLocked(const IpcStreamFrame& frame)
    {
        if (m_options.format == IpcWireFormat::Ndjson)
        {
            std::string line = SerializeFrameNdjson(frame);
            line.push_back('\n');
            return m_writer->WriteBytes(std::span<const std::uint8_t>(
                reinterpret_cast<const std::uint8_t*>(line.data()), line.size()));
        }

        const auto payload = SerializeFrameMessagePack(frame);
        const auto chunk = FrameMessagePackChunk(payload);
        return m_writer->WriteBytes(chunk);
    }

    // ── Pure helpers ─────────────────────────────────────────────────────────

    std::optional<std::wstring> NormalizePipePath(const std::wstring_view rawPath)
    {
        std::wstring_view view = TrimView(rawPath);
        while (view.size() >= 2 && view.front() == L'"' && view.back() == L'"')
        {
            view.remove_prefix(1);
            view.remove_suffix(1);
            view = TrimView(view);
        }
        if (view.empty()) return std::nullopt;

        std::wstring normalized(view);
        std::ranges::replace(normalized, L'/', L'\\');
        while (!normalized.empty() && normalized.back() == L'\\') normalized.pop_back();
        if (normalized.empty()) return std::nullopt;

        if (normalized.starts_with(L"\\\\"))
        {
            const std::size_t firstSep = normalized.find(L'\\', 2);
            if (firstSep == std::wstring::npos || firstSep == 2) return std::nullopt;
            const std::size_t secondSep = normalized.find(L'\\', firstSep + 1);
            if (secondSep == std::wstring::npos) return std::nullopt;

            const std::wstring_view host(normalized.data() + 2, firstSep - 2);
            const std::wstring_view pipeSegment(normalized.data() + firstSep + 1, secondSep - (firstSep + 1));
            const std::wstring_view name(normalized.data() + secondSep + 1);
            if (host.empty() || !EqualsCaseInsensitive(pipeSegment, L"pipe") ||
                name.empty() || name.find(L':') != std::wstring_view::npos)
            {
                return std::nullopt;
            }
            return std::wstring(L"\\\\") + std::wstring(host) + L"\\pipe\\" + std::wstring(name);
        }

        if (normalized.find(L':') != std::wstring::npos) return std::nullopt;
        std::wstring_view bare(normalized);
        while (!bare.empty() && bare.front() == L'\\') bare.remove_prefix(1);
        if (bare.empty()) return std::nullopt;
        return std::wstring(L"\\\\.\\pipe\\") + std::wstring(bare);
    }

    IpcFlagParseResult ParseIpcPipeFlag(std::wstring_view rawFlagBody)
    {
        rawFlagBody = TrimView(rawFlagBody);
        if (!rawFlagBody.empty() && rawFlagBody.front() == L'-')
        {
            rawFlagBody.remove_prefix(1);
        }

        const std::size_t delim = rawFlagBody.find_first_of(L":=");
        const std::wstring_view flagName = delim == std::wstring_view::npos
            ? rawFlagBody
            : rawFlagBody.substr(0, delim);

        if (!EqualsCaseInsensitive(flagName, L"ipcpipe"))
        {
            return {};
        }

        if (delim == std::wstring_view::npos)
        {
            return IpcFlagParseResult{
                .matched = true,
                .valid = true,
                .needsNextArg = true,
                .pipePath = std::nullopt,
            };
        }

        const std::wstring_view value = rawFlagBody.substr(delim + 1);
        auto normalized = NormalizePipePath(value);
        if (!normalized.has_value())
        {
            return IpcFlagParseResult{
                .matched = true,
                .valid = false,
                .needsNextArg = false,
                .pipePath = std::nullopt,
            };
        }

        return IpcFlagParseResult{
            .matched = true,
            .valid = true,
            .needsNextArg = false,
            .pipePath = std::move(normalized),
        };
    }

    std::string WideToUtf8(const std::wstring_view wide)
    {
        if (wide.empty()) return {};
        const int needed = WideCharToMultiByte(
            CP_UTF8, 0, wide.data(), static_cast<int>(wide.size()), nullptr, 0, nullptr, nullptr);
        if (needed <= 0) return {};
        std::string out(static_cast<std::size_t>(needed), '\0');
        WideCharToMultiByte(
            CP_UTF8, 0, wide.data(), static_cast<int>(wide.size()), out.data(), needed, nullptr, nullptr);
        return out;
    }

    std::wstring Utf8ToWide(const std::string_view utf8)
    {
        if (utf8.empty()) return {};
        const int needed = MultiByteToWideChar(
            CP_UTF8, 0, utf8.data(), static_cast<int>(utf8.size()), nullptr, 0);
        if (needed <= 0) return {};
        std::wstring out(static_cast<std::size_t>(needed), L'\0');
        MultiByteToWideChar(
            CP_UTF8, 0, utf8.data(), static_cast<int>(utf8.size()), out.data(), needed);
        return out;
    }

    std::string FileTimeToIso8601Utc(const std::uint64_t fileTime100ns)
    {
        if (fileTime100ns == 0) return {};
        FILETIME ft{};
        ft.dwLowDateTime = static_cast<DWORD>(fileTime100ns & 0xFFFFFFFFu);
        ft.dwHighDateTime = static_cast<DWORD>(fileTime100ns >> 32);
        SYSTEMTIME st{};
        if (!FileTimeToSystemTime(&ft, &st)) return {};
        char buf[32];
        std::snprintf(buf, sizeof(buf), "%04u-%02u-%02uT%02u:%02u:%02uZ",
            static_cast<unsigned>(st.wYear),
            static_cast<unsigned>(st.wMonth),
            static_cast<unsigned>(st.wDay),
            static_cast<unsigned>(st.wHour),
            static_cast<unsigned>(st.wMinute),
            static_cast<unsigned>(st.wSecond));
        return std::string(buf);
    }

    std::string SerializeNodeNdjson(const IpcNodeRecord& node)
    {
        std::string out;
        out.reserve(160 + node.name.size() + node.full_path.size());
        AppendNodeJson(out, node);
        return out;
    }

    std::string SerializeFrameNdjson(const IpcStreamFrame& frame)
    {
        std::string out;
        out.reserve(128 + frame.nodes.size() * 192);
        out += "{\"frame_type\":";
        AppendJsonEscaped(out, FrameTypeToString(frame.frame_type));
        out += ",\"seq\":";
        out += std::to_string(frame.seq);
        out += ",\"scan_root\":";
        AppendJsonEscaped(out, frame.scan_root);
        out += ",\"scanned_nodes\":";
        out += std::to_string(frame.scanned_nodes);
        out += ",\"total_bytes\":";
        out += std::to_string(frame.total_bytes);
        out += ",\"is_final\":";
        out += (frame.is_final ? "true" : "false");
        out += ",\"error\":";
        if (frame.error.has_value()) AppendJsonEscaped(out, *frame.error);
        else out += "null";
        out += ",\"nodes\":[";
        for (std::size_t i = 0; i < frame.nodes.size(); ++i)
        {
            if (i > 0) out.push_back(',');
            AppendNodeJson(out, frame.nodes[i]);
        }
        out += "]}";
        return out;
    }

    std::vector<std::uint8_t> SerializeFrameMessagePack(const IpcStreamFrame& frame)
    {
        MsgPackEncoder enc;
        enc.WriteMapHeader(8);
        enc.WriteKeyValue("frame_type", FrameTypeToString(frame.frame_type));
        enc.WriteKeyValue("seq", frame.seq);
        enc.WriteKeyValue("scan_root", frame.scan_root);
        enc.WriteKeyValue("scanned_nodes", frame.scanned_nodes);
        enc.WriteKeyValue("total_bytes", frame.total_bytes);
        enc.WriteKeyValue("is_final", frame.is_final);
        enc.WriteKeyOptionalStr("error", frame.error);

        enc.WriteStr("nodes");
        enc.WriteArrayHeader(frame.nodes.size());
        for (const auto& node : frame.nodes)
        {
            enc.WriteMapHeader(11);
            enc.WriteKeyValue("id", node.id);
            enc.WriteKeyOptionalUInt("parent_id", node.parent_id);
            enc.WriteKeyValue("name", node.name);
            enc.WriteKeyValue("full_path", node.full_path);
            enc.WriteKeyValue("is_dir", node.is_dir);
            enc.WriteKeyValue("logical_size", node.logical_size);
            enc.WriteKeyValue("physical_size", node.physical_size);
            enc.WriteKeyValue("modified_utc", node.modified_utc);
            enc.WriteKeyValue("owner", node.owner);
            enc.WriteKeyOptionalUInt("xxhash64", node.xxhash64);
            enc.WriteKeyValue("extension", node.extension);
        }
        return enc.Take();
    }

    std::vector<std::uint8_t> FrameMessagePackChunk(const std::span<const std::uint8_t> msgpackPayload)
    {
        const std::uint32_t len = static_cast<std::uint32_t>(msgpackPayload.size());
        std::vector<std::uint8_t> out;
        out.reserve(8 + msgpackPayload.size());
        out.insert(out.end(), std::begin(kMsgPackChunkMagic), std::end(kMsgPackChunkMagic));
        out.push_back(static_cast<std::uint8_t>(len & 0xFFu));
        out.push_back(static_cast<std::uint8_t>((len >> 8) & 0xFFu));
        out.push_back(static_cast<std::uint8_t>((len >> 16) & 0xFFu));
        out.push_back(static_cast<std::uint8_t>((len >> 24) & 0xFFu));
        out.insert(out.end(), msgpackPayload.begin(), msgpackPayload.end());
        return out;
    }
}

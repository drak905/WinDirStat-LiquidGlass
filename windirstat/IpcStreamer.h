// WinDirStat - Windows Directory Statistics
// Copyright © WinDirStat Team
//
// SPDX-License-Identifier: GPL-3.0-or-later
// Distributed WITHOUT ANY WARRANTY; see LICENSE.md for details.

#pragma once

#include <chrono>
#include <cstdint>
#include <memory>
#include <mutex>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <vector>

namespace wds::ipc
{
    inline constexpr std::wstring_view kDefaultPipePath = L"\\\\.\\pipe\\wds_liquid_glass";
    inline constexpr std::uint32_t kDefaultTimeoutMs = 5000;
    inline constexpr std::uint8_t kMsgPackChunkMagic[4] = { 'W', 'D', 'S', 'M' };

    enum class IpcWireFormat : std::uint8_t
    {
        Ndjson = 0,
        MessagePack = 1,
    };

    enum class IpcFrameType : std::uint8_t
    {
        Nodes = 0,
        Complete = 1,
        Error = 2,
    };

    struct IpcNodeRecord final
    {
        std::uint64_t id = 0;
        std::optional<std::uint64_t> parent_id;
        std::string name;
        std::string full_path;
        bool is_dir = false;
        std::uint64_t logical_size = 0;
        std::uint64_t physical_size = 0;
        std::string modified_utc;
        std::string owner;
        std::optional<std::uint64_t> xxhash64;
        std::string extension;
    };

    struct IpcStreamFrame final
    {
        IpcFrameType frame_type = IpcFrameType::Nodes;
        std::uint64_t seq = 0;
        std::string scan_root;
        std::uint64_t scanned_nodes = 0;
        std::uint64_t total_bytes = 0;
        bool is_final = false;
        std::optional<std::string> error;
        std::vector<IpcNodeRecord> nodes;
    };

    struct IpcFlagParseResult final
    {
        bool matched = false;
        bool valid = false;
        bool needsNextArg = false;
        std::optional<std::wstring> pipePath;
    };

    class IIpcWriter
    {
    public:
        virtual ~IIpcWriter() = default;
        virtual bool IsOpen() const noexcept = 0;
        virtual bool WriteBytes(std::span<const std::uint8_t> data) = 0;
        virtual bool Flush() = 0;
        virtual void Close() noexcept = 0;
    };

    class IpcBufferWriter final : public IIpcWriter
    {
    public:
        bool IsOpen() const noexcept override;
        bool WriteBytes(std::span<const std::uint8_t> data) override;
        bool Flush() override;
        void Close() noexcept override;

        const std::vector<std::uint8_t>& Bytes() const noexcept { return m_buffer; }
        std::string_view AsStringView() const noexcept;
        void Clear() noexcept;

    private:
        mutable std::mutex m_mutex;
        std::vector<std::uint8_t> m_buffer;
        bool m_open = true;
    };

    class IpcNamedPipeWriter final : public IIpcWriter
    {
    public:
        explicit IpcNamedPipeWriter(std::wstring pipePath, std::uint32_t connectTimeoutMs = kDefaultTimeoutMs);
        ~IpcNamedPipeWriter() override;

        IpcNamedPipeWriter(const IpcNamedPipeWriter&) = delete;
        IpcNamedPipeWriter& operator=(const IpcNamedPipeWriter&) = delete;
        IpcNamedPipeWriter(IpcNamedPipeWriter&& other) noexcept;
        IpcNamedPipeWriter& operator=(IpcNamedPipeWriter&& other) noexcept;

        bool IsOpen() const noexcept override;
        bool WriteBytes(std::span<const std::uint8_t> data) override;
        bool Flush() override;
        void Close() noexcept override;

        const std::wstring& PipePath() const noexcept { return m_pipePath; }
        std::uint32_t LastError() const noexcept { return m_lastError; }

    private:
        bool Connect(std::uint32_t connectTimeoutMs);

        std::wstring m_pipePath;
        void* m_handle = nullptr;
        std::uint32_t m_lastError = 0;
    };

    struct IpcStreamerOptions final
    {
        IpcWireFormat format = IpcWireFormat::Ndjson;
        std::size_t batch_size = 256;
        std::uint32_t flush_interval_ms = 16; // ~60 FPS progressive batches
        std::string scan_root;
    };

    class IpcStreamer final
    {
    public:
        IpcStreamer(std::shared_ptr<IIpcWriter> writer, IpcStreamerOptions options = {});

        bool PushNode(IpcNodeRecord node);
        bool FlushBatch();
        bool Finish(std::string_view errorMessage = {});

        std::uint64_t ScannedNodes() const noexcept;
        std::uint64_t TotalBytes() const noexcept;
        std::uint64_t FramesSent() const noexcept;

    private:
        bool FlushBatchLocked();
        bool WriteFrameLocked(const IpcStreamFrame& frame);

        std::shared_ptr<IIpcWriter> m_writer;
        IpcStreamerOptions m_options;
        mutable std::mutex m_mutex;
        std::vector<IpcNodeRecord> m_pendingNodes;
        std::uint64_t m_seq = 0;
        std::uint64_t m_scannedNodes = 0;
        std::uint64_t m_totalBytes = 0;
        bool m_finished = false;
        std::chrono::steady_clock::time_point m_lastFlush;
    };

    // Pure helpers (testable outside UI/model code)
    std::optional<std::wstring> NormalizePipePath(std::wstring_view rawPath);
    IpcFlagParseResult ParseIpcPipeFlag(std::wstring_view rawFlagBody);

    std::string WideToUtf8(std::wstring_view wide);
    std::wstring Utf8ToWide(std::string_view utf8);
    std::string FileTimeToIso8601Utc(std::uint64_t fileTime100ns);

    std::string SerializeNodeNdjson(const IpcNodeRecord& node);
    std::string SerializeFrameNdjson(const IpcStreamFrame& frame);
    std::vector<std::uint8_t> SerializeFrameMessagePack(const IpcStreamFrame& frame);
    std::vector<std::uint8_t> FrameMessagePackChunk(std::span<const std::uint8_t> msgpackPayload);
}

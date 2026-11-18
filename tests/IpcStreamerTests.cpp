// WinDirStat - Windows Directory Statistics
// Copyright © WinDirStat Team
//
// SPDX-License-Identifier: GPL-3.0-or-later
// Distributed WITHOUT ANY WARRANTY; see LICENSE.md for details.

#include "IpcStreamer.h"

#include <windows.h>
#include <cassert>
#include <cstdint>
#include <cstring>
#include <iostream>
#include <memory>
#include <string>
#include <string_view>
#include <thread>
#include <vector>

namespace
{
    void Require(const bool cond, const char* msg)
    {
        if (!cond)
        {
            std::cerr << "FAIL: " << msg << '\n';
            std::exit(1);
        }
    }

    std::vector<std::string> SplitLines(std::string_view text)
    {
        std::vector<std::string> lines;
        size_t pos = 0;
        while (pos < text.size())
        {
            const size_t nl = text.find('\n', pos);
            const size_t end = (nl == std::string_view::npos) ? text.size() : nl;
            auto line = text.substr(pos, end - pos);
            if (!line.empty() && line.back() == '\r') line.remove_suffix(1);
            if (!line.empty()) lines.emplace_back(line);
            if (nl == std::string_view::npos) break;
            pos = nl + 1;
        }
        return lines;
    }

    std::uint32_t ReadU32Le(const std::uint8_t* p)
    {
        return static_cast<std::uint32_t>(p[0]) |
               (static_cast<std::uint32_t>(p[1]) << 8) |
               (static_cast<std::uint32_t>(p[2]) << 16) |
               (static_cast<std::uint32_t>(p[3]) << 24);
    }

    void TestPipePathNormalization()
    {
        using wds::ipc::NormalizePipePath;

        Require(NormalizePipePath(L"wds_liquid_glass") == L"\\\\.\\pipe\\wds_liquid_glass",
            "bare pipe name should normalize to \\\\.\\pipe\\...");
        Require(NormalizePipePath(L"\\\\.\\pipe\\wds_liquid_glass") == L"\\\\.\\pipe\\wds_liquid_glass",
            "canonical pipe path should remain unchanged");
        Require(NormalizePipePath(L"//./pipe/wds_liquid_glass") == L"\\\\.\\pipe\\wds_liquid_glass",
            "forward slashes should normalize to backslashes");
        Require(NormalizePipePath(L"\\\\.\\PIPE\\wds_liquid_glass") == L"\\\\.\\pipe\\wds_liquid_glass",
            "case-insensitive \\PIPE\\ segment should normalize");
        Require(NormalizePipePath(L"\"\\\\.\\pipe\\wds_liquid_glass\\\"") == L"\\\\.\\pipe\\wds_liquid_glass",
            "quotes and trailing backslashes should be stripped");
        Require(NormalizePipePath(L"\\\\fileserver\\pipe\\wds_pipe") == L"\\\\fileserver\\pipe\\wds_pipe",
            "UNC server pipe path should be preserved");
        Require(!NormalizePipePath(L"").has_value(), "empty pipe path should fail");
        Require(!NormalizePipePath(L"   ").has_value(), "whitespace pipe path should fail");
        Require(!NormalizePipePath(L"\\\\.\\pipe\\").has_value(), "pipe prefix without name should fail");
        Require(!NormalizePipePath(L"C:\\temp\\not_a_pipe").has_value(), "drive path should not normalize to pipe");
    }

    void TestFlagParsing()
    {
        using wds::ipc::ParseIpcPipeFlag;

        {
            const auto r = ParseIpcPipeFlag(L"ipcpipe");
            Require(r.matched && r.valid && r.needsNextArg && !r.pipePath.has_value(),
                "two-arg /ipcpipe should need next arg");
        }
        {
            const auto r = ParseIpcPipeFlag(L"IpcPipe:\\\\.\\pipe\\wds_liquid_glass");
            Require(r.matched && r.valid && !r.needsNextArg &&
                r.pipePath == L"\\\\.\\pipe\\wds_liquid_glass",
                "colon-delimited /IpcPipe:<path> should parse pipe path");
        }
        {
            const auto r = ParseIpcPipeFlag(L"ipcpipe=wds_custom");
            Require(r.matched && r.valid && !r.needsNextArg &&
                r.pipePath == L"\\\\.\\pipe\\wds_custom",
                "equals-delimited -ipcpipe=<name> should normalize pipe path");
        }
        {
            const auto r = ParseIpcPipeFlag(L"IpcPipe:");
            Require(r.matched && !r.valid, "empty inline /IpcPipe: should be invalid");
        }
        {
            const auto r = ParseIpcPipeFlag(L"saveto");
            Require(!r.matched, "unrelated flag should not match ipcpipe");
        }
    }

    void TestNdjsonNodeSerialization()
    {
        wds::ipc::IpcNodeRecord node{
            .id = 42,
            .parent_id = 7,
            .name = "quote\"file\\name.txt",
            .full_path = "C:\\Users\\Alice\\quote\"file\\name.txt",
            .is_dir = false,
            .logical_size = 123456,
            .physical_size = 131072,
            .modified_utc = "2026-11-18T13:00:00Z",
            .owner = "BUILTIN\\Administrators",
            .xxhash64 = 0xDEADBEEFCAFEBABEULL,
            .extension = ".txt"
        };

        const std::string json = wds::ipc::SerializeNodeNdjson(node);
        Require(json.find("\"id\":42") != std::string::npos, "id field present");
        Require(json.find("\"parent_id\":7") != std::string::npos, "parent_id field present");
        Require(json.find("\"name\":\"quote\\\"file\\\\name.txt\"") != std::string::npos, "escaped name");
        Require(json.find("\"full_path\":\"C:\\\\Users\\\\Alice\\\\quote\\\"file\\\\name.txt\"") != std::string::npos, "escaped path");
        Require(json.find("\"is_dir\":false") != std::string::npos, "is_dir false");
        Require(json.find("\"logical_size\":123456") != std::string::npos, "logical_size");
        Require(json.find("\"physical_size\":131072") != std::string::npos, "physical_size");
        Require(json.find("\"modified_utc\":\"2026-11-18T13:00:00Z\"") != std::string::npos, "modified_utc");
        Require(json.find("\"owner\":\"BUILTIN\\\\Administrators\"") != std::string::npos, "owner");
        Require(json.find("\"xxhash64\":" + std::to_string(0xDEADBEEFCAFEBABEULL)) != std::string::npos, "xxhash64 u64");
        Require(json.find("\"extension\":\".txt\"") != std::string::npos, "extension");

        wds::ipc::IpcNodeRecord root{
            .id = 1,
            .parent_id = std::nullopt,
            .name = "C:\\",
            .full_path = "C:\\",
            .is_dir = true,
            .logical_size = 0,
            .physical_size = 0,
            .modified_utc = "",
            .owner = "",
            .xxhash64 = std::nullopt,
            .extension = ""
        };
        const std::string rootJson = wds::ipc::SerializeNodeNdjson(root);
        Require(rootJson.find("\"parent_id\":null") != std::string::npos, "null parent_id");
        Require(rootJson.find("\"xxhash64\":null") != std::string::npos, "null xxhash64");
        Require(rootJson.find("\"is_dir\":true") != std::string::npos, "is_dir true");
    }

    void TestProgressiveNdjsonStreaming()
    {
        auto writer = std::make_shared<wds::ipc::IpcBufferWriter>();
        wds::ipc::IpcStreamerOptions opts{
            .format = wds::ipc::IpcWireFormat::Ndjson,
            .batch_size = 2,
            .flush_interval_ms = 0,
            .scan_root = "C:\\Test"
        };
        wds::ipc::IpcStreamer streamer(writer, opts);

        for (std::uint64_t i = 1; i <= 5; ++i)
        {
            Require(streamer.PushNode({
                .id = i,
                .parent_id = (i == 1 ? std::nullopt : std::optional<std::uint64_t>(1)),
                .name = "item_" + std::to_string(i),
                .full_path = "C:\\Test\\item_" + std::to_string(i),
                .is_dir = (i == 1),
                .logical_size = i * 100,
                .physical_size = i * 128,
                .modified_utc = "2026-11-18T10:00:00Z",
                .owner = "",
                .xxhash64 = std::nullopt,
                .extension = (i == 1 ? "" : ".bin")
            }), "PushNode succeeded");
        }

        // Before Finish: 2 batches of 2 should have been flushed (4 nodes)
        {
            const auto lines = SplitLines(writer->AsStringView());
            Require(lines.size() == 2, "expected 2 flushed batch frames before Finish");
            Require(lines[0].find("\"frame_type\":\"nodes\"") != std::string::npos, "frame 0 is nodes");
            Require(lines[0].find("\"seq\":1") != std::string::npos, "frame 0 seq 1");
            Require(lines[1].find("\"seq\":2") != std::string::npos, "frame 1 seq 2");
        }

        Require(streamer.Finish(), "Finish succeeded");

        const auto lines = SplitLines(writer->AsStringView());
        Require(lines.size() == 4, "expected 3 node batches + 1 final complete frame");
        Require(lines[2].find("\"seq\":3") != std::string::npos, "frame 2 seq 3");
        Require(lines[2].find("item_5") != std::string::npos, "frame 2 has 5th item");
        Require(lines[3].find("\"frame_type\":\"complete\"") != std::string::npos, "final frame is complete");
        Require(lines[3].find("\"is_final\":true") != std::string::npos, "final frame has is_final:true");
        Require(lines[3].find("\"scanned_nodes\":5") != std::string::npos, "final frame scanned_nodes=5");
        Require(lines[3].find("\"total_bytes\":1400") != std::string::npos, "final frame total_bytes=1400 (sum of non-dir files 200+300+400+500)");

        // Verify error frame + UTF-8/control char escaping
        auto errWriter = std::make_shared<wds::ipc::IpcBufferWriter>();
        wds::ipc::IpcStreamer errStreamer(errWriter, {
            .format = wds::ipc::IpcWireFormat::Ndjson,
            .batch_size = 8,
            .flush_interval_ms = 0,
            .scan_root = "C:\\UTF8"
        });
        Require(errStreamer.PushNode({
            .id = 1,
            .parent_id = std::nullopt,
            .name = "t\xE1\xBB\x87p\t\n.log",
            .full_path = "C:\\UTF8\\t\xE1\xBB\x87p\t\n.log",
            .is_dir = false,
            .logical_size = 64,
            .physical_size = 64,
            .modified_utc = "",
            .owner = "",
            .xxhash64 = std::nullopt,
            .extension = ".log"
        }), "PushNode UTF-8");
        Require(errStreamer.Finish("access denied"), "Finish with error");
        const auto errLines = SplitLines(errWriter->AsStringView());
        Require(errLines.size() == 2, "expected 1 node batch + 1 error frame");
        Require(errLines[0].find("t\xE1\xBB\x87p\\t\\n.log") != std::string::npos, "UTF-8 preserved and control chars escaped");
        Require(errLines[1].find("\"frame_type\":\"error\"") != std::string::npos, "error frame type");
        Require(errLines[1].find("\"error\":\"access denied\"") != std::string::npos, "error frame message");
    }

    void TestMessagePackChunkFraming()
    {
        auto writer = std::make_shared<wds::ipc::IpcBufferWriter>();
        wds::ipc::IpcStreamerOptions opts{
            .format = wds::ipc::IpcWireFormat::MessagePack,
            .batch_size = 2,
            .flush_interval_ms = 0,
            .scan_root = "D:\\Work"
        };
        wds::ipc::IpcStreamer streamer(writer, opts);

        Require(streamer.PushNode({
            .id = 1,
            .parent_id = std::nullopt,
            .name = "Work",
            .full_path = "D:\\Work",
            .is_dir = true,
            .logical_size = 512,
            .physical_size = 4096,
            .modified_utc = "2026-11-18T12:34:56Z",
            .owner = "User",
            .xxhash64 = std::nullopt,
            .extension = ""
        }), "PushNode dir");

        Require(streamer.PushNode({
            .id = 2,
            .parent_id = 1,
            .name = "data.bin",
            .full_path = "D:\\Work\\data.bin",
            .is_dir = false,
            .logical_size = 512,
            .physical_size = 4096,
            .modified_utc = "2026-11-18T12:34:56Z",
            .owner = "User",
            .xxhash64 = 0x1122334455667788ULL,
            .extension = ".bin"
        }), "PushNode file");

        Require(streamer.Finish(), "Finish msgpack");

        const auto& bytes = writer->Bytes();
        Require(bytes.size() > 16, "msgpack output should not be empty");

        // Parse 2 length-prefixed chunks: "WDSM" + u32_le + msgpack map
        size_t offset = 0;
        int chunkCount = 0;
        while (offset < bytes.size())
        {
            Require(offset + 8 <= bytes.size(), "chunk header fits");
            Require(std::memcmp(bytes.data() + offset, "WDSM", 4) == 0, "magic WDSM matches");
            const std::uint32_t payloadLen = ReadU32Le(bytes.data() + offset + 4);
            offset += 8;
            Require(payloadLen > 0 && offset + payloadLen <= bytes.size(), "payload bounds valid");
            // Top-level msgpack object must be a fixmap (0x80..0x8F) or map16 (0xDE)
            const std::uint8_t firstByte = bytes[offset];
            Require((firstByte & 0xF0) == 0x80 || firstByte == 0xDE, "msgpack payload begins with map");
            offset += payloadLen;
            ++chunkCount;
        }
        Require(chunkCount == 2, "expected 2 msgpack frames (1 node batch + 1 complete)");
    }

    void TestFileTimeConversion()
    {
        // 1970-01-01T00:00:00Z in 100ns intervals since 1601-01-01
        Require(wds::ipc::FileTimeToIso8601Utc(116444736000000000ULL) == "1970-01-01T00:00:00Z",
            "Unix epoch converts cleanly");
        Require(wds::ipc::FileTimeToIso8601Utc(0ULL).empty(),
            "zero FILETIME returns empty string");
    }

    void TestRealWindowsNamedPipeRoundTrip()
    {
        const std::wstring pipeName = L"\\\\.\\pipe\\wds_cpp_ipc_test_" + std::to_wstring(GetCurrentProcessId());

        HANDLE serverPipe = CreateNamedPipeW(
            pipeName.c_str(),
            PIPE_ACCESS_INBOUND,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            1,
            65536,
            65536,
            5000,
            nullptr);
        Require(serverPipe != INVALID_HANDLE_VALUE, "CreateNamedPipeW succeeded");

        std::string received;
        bool connectOk = false;
        std::thread readerThread([&] {
            connectOk = ConnectNamedPipe(serverPipe, nullptr) != 0 || GetLastError() == ERROR_PIPE_CONNECTED;
            if (connectOk)
            {
                char buf[4096];
                DWORD bytesRead = 0;
                while (ReadFile(serverPipe, buf, sizeof(buf), &bytesRead, nullptr) && bytesRead > 0)
                {
                    received.append(buf, bytesRead);
                }
            }
            DisconnectNamedPipe(serverPipe);
            CloseHandle(serverPipe);
        });

        auto pipeWriter = std::make_shared<wds::ipc::IpcNamedPipeWriter>(pipeName, 3000);
        Require(pipeWriter->IsOpen(), "IpcNamedPipeWriter connected to pipe");

        wds::ipc::IpcStreamer streamer(pipeWriter, {
            .format = wds::ipc::IpcWireFormat::Ndjson,
            .batch_size = 16,
            .flush_interval_ms = 0,
            .scan_root = "C:\\PipeTest"
        });
        Require(streamer.PushNode({
            .id = 1,
            .parent_id = std::nullopt,
            .name = "hello.txt",
            .full_path = "C:\\PipeTest\\hello.txt",
            .is_dir = false,
            .logical_size = 1024,
            .physical_size = 4096,
            .modified_utc = "2026-11-18T12:00:00Z",
            .owner = "",
            .xxhash64 = 99999ULL,
            .extension = ".txt"
        }), "PushNode over pipe");
        Require(streamer.Finish(), "Finish over pipe");
        pipeWriter->Close();

        readerThread.join();
        Require(connectOk, "pipe server accepted connection");
        const auto lines = SplitLines(received);
        Require(lines.size() == 2, "pipe received batch + complete frame");
        Require(lines[0].find("hello.txt") != std::string::npos, "pipe payload contains node");
        Require(lines[1].find("\"is_final\":true") != std::string::npos, "pipe payload contains complete frame");
    }
}

int main()
{
    TestPipePathNormalization();
    TestFlagParsing();
    TestNdjsonNodeSerialization();
    TestProgressiveNdjsonStreaming();
    TestMessagePackChunkFraming();
    TestFileTimeConversion();
    TestRealWindowsNamedPipeRoundTrip();
    std::cout << "ALL C++23 IpcStreamerTests PASSED (7/7)\n";
    return 0;
}

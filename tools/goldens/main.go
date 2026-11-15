// Command goldens writes the golden byte vectors the Rust codecs are tested against
// (testdata/goldens/*.hex). Every byte comes from upstream Go: message bodies from the
// generated Pack methods, frames as go/client writes them (client.go, registryreq.go,
// blocksreq.go), CRCs from go/core/crc32c, Reed-Solomon parity and recovery from go/core/rs.
// Output is deterministic.
//
// Line format: `<hex> # <description>`; frames.hex: `<frame kind> <hex> # <description>`;
// crc32c.hex: `<op> <fields...> # <description>`; rs.hex: see rs.go; messages.hex (every struct, filled by
// reflection): `<type> <hex> # <description>`; names.txt
// (message kinds and error codes): `<set> <value> <NAME>`. `-` stands for empty bytes, lines
// starting with `#` are comments.
//
// Run from tools/: go run ./goldens ../testdata/goldens
package main

import (
	"bytes"
	"encoding/binary"
	"encoding/hex"
	"fmt"
	"math/rand"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/XTXMarkets/ternfs/go/core/bincode"
	"github.com/XTXMarkets/ternfs/go/core/crc32c"
	"github.com/XTXMarkets/ternfs/go/core/parity"
	"github.com/XTXMarkets/ternfs/go/msgs"
)

const mtu = int(msgs.DEFAULT_UDP_MTU)

// Shard/CDC frame header: u32 protocol, u64 request id, u8 kind.
const shardHeader = 4 + 8 + 1

type sample struct {
	v    bincode.Packable
	desc string
}

var rng = rand.New(rand.NewSource(0x7e4f5))

func pack(v bincode.Packable) []byte {
	var buf bytes.Buffer
	if err := v.Pack(&buf); err != nil {
		panic(fmt.Sprintf("pack %T: %v", v, err))
	}
	return buf.Bytes()
}

func hexOrDash(b []byte) string {
	if len(b) == 0 {
		return "-"
	}
	return hex.EncodeToString(b)
}

func randBytes(n int) []byte {
	b := make([]byte, n)
	rng.Read(b)
	return b
}

func fileId(shard uint8, n uint64) msgs.InodeId {
	return msgs.InodeId(uint64(msgs.FILE)<<61 | n<<8 | uint64(shard))
}

func dirId(shard uint8, n uint64) msgs.InodeId {
	return msgs.InodeId(uint64(msgs.DIRECTORY)<<61 | n<<8 | uint64(shard))
}

func ipPort(a, b, c, d byte, port uint16) msgs.IpPort {
	return msgs.IpPort{Addrs: msgs.Ip{a, b, c, d}, Port: port}
}

func name(n int, seed byte) string {
	b := make([]byte, n)
	for i := range b {
		b[i] = 'a' + (seed+byte(i))%26
	}
	return string(b)
}

const nonUtf8 = "\xff\xfe\xc3\x28bad\x80"

func blockService(i int) msgs.BlockService {
	return msgs.BlockService{
		Addrs: msgs.AddrsInfo{Addr1: ipPort(10, 0, 0, byte(i), 5000+uint16(i)), Addr2: ipPort(10, 1, 0, byte(i), 6000+uint16(i))},
		Id:    msgs.BlockServiceId(0x1000_0000_0000 + uint64(i)),
		Flags: msgs.BlockServiceFlags(0),
	}
}

func blocksSpan(off uint64, size uint32, d, p uint8, stripes uint8, cell uint32) msgs.FetchedSpan {
	nb := int(d) + int(p)
	blocks := make([]msgs.FetchedBlock, nb)
	for i := range blocks {
		blocks[i] = msgs.FetchedBlock{BlockServiceIx: uint8(i), BlockId: msgs.BlockId(0xb10c_0000 + uint64(i)), Crc: msgs.Crc(rng.Uint32())}
	}
	crcs := make([]msgs.Crc, stripes)
	for i := range crcs {
		crcs[i] = msgs.Crc(rng.Uint32())
	}
	return msgs.FetchedSpan{
		Header: msgs.FetchedSpanHeader{ByteOffset: off, Size: size, Crc: msgs.Crc(rng.Uint32()), StorageClass: msgs.HDD_STORAGE},
		Body:   &msgs.FetchedBlocksSpan{Parity: parity.MkParity(d, p), Stripes: stripes, CellSize: cell, Blocks: blocks, StripesCrc: crcs},
	}
}

func inlineSpan(off uint64, body []byte) msgs.FetchedSpan {
	return msgs.FetchedSpan{
		Header: msgs.FetchedSpanHeader{ByteOffset: off, Size: uint32(len(body)), Crc: msgs.Crc(crc32c.Sum(0, body)), StorageClass: msgs.INLINE_STORAGE},
		Body:   &msgs.FetchedInlineSpan{Body: body},
	}
}

// maxReadDir returns the largest ReadDirResp whose shard response frame fits the MTU.
func maxReadDir() *msgs.ReadDirResp {
	resp := &msgs.ReadDirResp{NextHash: 0x1234}
	for i := 0; ; i++ {
		e := msgs.CurrentEdge{TargetId: fileId(uint8(i), uint64(i)), NameHash: msgs.NameHash(i), Name: name(8, byte(i)), CreationTime: msgs.TernTime(1_700_000_000_000_000_000 + i)}
		resp.Results = append(resp.Results, e)
		if shardHeader+len(pack(resp)) > mtu {
			resp.Results = resp.Results[:len(resp.Results)-1]
			return resp
		}
	}
}

// maxInlineSpans returns the largest LocalFileSpansResp of 16-byte inline spans that fits.
func maxInlineSpans() *msgs.LocalFileSpansResp {
	resp := &msgs.LocalFileSpansResp{NextOffset: 0}
	for i := 0; ; i++ {
		resp.Spans = append(resp.Spans, inlineSpan(uint64(i)*16, randBytes(16)))
		if shardHeader+len(pack(resp)) > mtu {
			resp.Spans = resp.Spans[:len(resp.Spans)-1]
			return resp
		}
	}
}

func messages() map[string][]sample {
	shards := make([]msgs.ShardInfo, 256)
	for i := range shards {
		shards[i] = msgs.ShardInfo{
			Addrs:    msgs.AddrsInfo{Addr1: ipPort(127, 0, 0, 1, 20000+uint16(i)), Addr2: ipPort(127, 0, 1, 1, 30000+uint16(i))},
			LastSeen: msgs.TernTime(1_700_000_000_000_000_000 + uint64(i)),
		}
	}
	zeroShards := make([]msgs.ShardInfo, 256)

	dirInfo := msgs.DirectoryInfo{Entries: []msgs.DirectoryInfoEntry{
		{Tag: msgs.DirectoryInfoTag(1), Body: randBytes(12)},
		{Tag: msgs.DirectoryInfoTag(2), Body: []byte{}},
	}}

	return map[string][]sample{
		"LocalShardsReq": {{&msgs.LocalShardsReq{}, "empty request"}},
		"LocalShardsResp": {
			{&msgs.LocalShardsResp{Shards: shards}, "256 shards, two addresses each"},
			{&msgs.LocalShardsResp{Shards: zeroShards}, "256 zeroed shards"},
			{&msgs.LocalShardsResp{}, "empty list"},
		},
		"LookupReq": {
			{&msgs.LookupReq{DirId: msgs.ROOT_DIR_INODE_ID, Name: "hello.txt"}, "root, short name"},
			{&msgs.LookupReq{DirId: dirId(7, 42), Name: name(255, 3)}, "255-byte name"},
			{&msgs.LookupReq{DirId: dirId(255, 1), Name: nonUtf8}, "non-UTF-8 name"},
			{&msgs.LookupReq{DirId: dirId(0, 1), Name: ""}, "empty name"},
		},
		"LookupResp": {
			{&msgs.LookupResp{TargetId: fileId(3, 99), CreationTime: msgs.TernTime(1_700_000_000_123_456_789)}, "file target"},
			{&msgs.LookupResp{TargetId: msgs.InodeId(uint64(msgs.SYMLINK)<<61 | 5), CreationTime: 0}, "symlink target, zero time"},
		},
		"StatFileReq": {{&msgs.StatFileReq{Id: fileId(9, 1234)}, "file id"}},
		"StatFileResp": {
			{&msgs.StatFileResp{Mtime: 1_700_000_000_000_000_001, Atime: 1_700_000_000_000_000_002, Size: 1 << 40}, "1 TiB file"},
			{&msgs.StatFileResp{}, "zero"},
			{&msgs.StatFileResp{Mtime: ^msgs.TernTime(0), Atime: ^msgs.TernTime(0), Size: ^uint64(0)}, "all ones"},
		},
		"StatDirectoryReq": {{&msgs.StatDirectoryReq{Id: msgs.ROOT_DIR_INODE_ID}, "root"}},
		"StatDirectoryResp": {
			{&msgs.StatDirectoryResp{Mtime: 1_700_000_000_000_000_000, Owner: dirId(1, 2), Info: dirInfo}, "owned, two info entries"},
			{&msgs.StatDirectoryResp{Mtime: 5, Owner: msgs.NULL_INODE_ID}, "snapshot, empty info"},
		},
		"ReadDirReq": {
			{&msgs.ReadDirReq{DirId: msgs.ROOT_DIR_INODE_ID, StartHash: 0, Mtu: msgs.DEFAULT_UDP_MTU}, "root from start"},
			{&msgs.ReadDirReq{DirId: dirId(4, 4), StartHash: 0xdead_beef_cafe, Mtu: 0}, "continuation, default MTU"},
		},
		"ReadDirResp": {
			{&msgs.ReadDirResp{NextHash: 0}, "empty, last page"},
			{&msgs.ReadDirResp{NextHash: 77, Results: []msgs.CurrentEdge{
				{TargetId: fileId(1, 1), NameHash: 11, Name: name(255, 0), CreationTime: 1},
				{TargetId: dirId(2, 2), NameHash: 12, Name: nonUtf8, CreationTime: 2},
			}}, "255-byte and non-UTF-8 names"},
			{maxReadDir(), "most 8-byte-name entries that fit 1472"},
		},
		"LocalFileSpansReq": {
			{&msgs.LocalFileSpansReq{FileId: fileId(5, 5), ByteOffset: 0, Limit: 0, Mtu: msgs.DEFAULT_UDP_MTU}, "from start"},
			{&msgs.LocalFileSpansReq{FileId: fileId(5, 5), ByteOffset: 1 << 30, Limit: 10, Mtu: 0}, "offset 1 GiB, limit 10"},
		},
		"LocalFileSpansResp": {
			{&msgs.LocalFileSpansResp{}, "empty"},
			{&msgs.LocalFileSpansResp{NextOffset: 0, Spans: []msgs.FetchedSpan{inlineSpan(0, randBytes(255))}}, "one 255-byte inline span"},
			{&msgs.LocalFileSpansResp{NextOffset: 0, Spans: []msgs.FetchedSpan{inlineSpan(0, []byte{})}}, "empty inline span"},
			{&msgs.LocalFileSpansResp{
				NextOffset:    1 << 20,
				BlockServices: []msgs.BlockService{blockService(0), blockService(1)},
				Spans:         []msgs.FetchedSpan{blocksSpan(0, 1<<20, 1, 1, 1, 1<<20)},
			}, "mirrored D=1 P=1, next offset set"},
			{&msgs.LocalFileSpansResp{
				NextOffset: 0,
				BlockServices: func() []msgs.BlockService {
					r := make([]msgs.BlockService, 14)
					for i := range r {
						r[i] = blockService(i)
					}
					return r
				}(),
				Spans: []msgs.FetchedSpan{inlineSpan(0, randBytes(3)), blocksSpan(3, 10<<20, 10, 4, 4, 1<<18)},
			}, "inline then RS D=10 P=4 with 4 stripes"},
			{maxInlineSpans(), "most 16-byte inline spans that fit 1472"},
		},
		"FileSpansReq": {{&msgs.FileSpansReq{FileId: fileId(5, 5), ByteOffset: 4096, Limit: 1, Mtu: msgs.DEFAULT_UDP_MTU}, "one span"}},
		"FileSpansResp": {
			{&msgs.FileSpansResp{Spans: []msgs.FetchedFullSpan{{
				Header: msgs.FetchedSpanHeaderFull{ByteOffset: 0, Size: 4, Crc: msgs.Crc(crc32c.Sum(0, []byte("abcd"))), IsInline: true},
				Body:   &msgs.FetchedInlineSpan{Body: []byte("abcd")},
			}}}, "inline full span"},
			{&msgs.FileSpansResp{
				BlockServices: []msgs.BlockService{blockService(0), blockService(1)},
				Spans: []msgs.FetchedFullSpan{{
					Header: msgs.FetchedSpanHeaderFull{ByteOffset: 0, Size: 8192, Crc: 7, IsInline: false},
					Body: &msgs.FetchedLocations{Locations: []msgs.FetchedBlockServices{{
						LocationId: 0, StorageClass: msgs.FLASH_STORAGE, Parity: parity.MkParity(1, 1), Stripes: 1, CellSize: 8192,
						Blocks:     []msgs.FetchedBlock{{BlockServiceIx: 0, BlockId: 1, Crc: 2}, {BlockServiceIx: 1, BlockId: 3, Crc: 4}},
						StripesCrc: []msgs.Crc{9},
					}}},
				}},
			}, "one location, mirrored"},
		},
		"FetchBlockWithCrcReq": {
			{&msgs.FetchBlockWithCrcReq{BlockId: 0xb10c, Offset: 0, Count: 4096}, "first page"},
			{&msgs.FetchBlockWithCrcReq{FileIdUnused: fileId(1, 1), BlockId: ^msgs.BlockId(0), BlockCrcUnused: 5, Offset: 1 << 20, Count: 256 * 4096}, "1 MiB at 1 MiB, unused fields set"},
		},
		"FetchBlockWithCrcResp": {{&msgs.FetchBlockWithCrcResp{}, "empty body (pages follow the frame)"}},
	}
}

func shardFrame(protocol uint32, reqId uint64, kind uint8, body []byte) []byte {
	b := binary.LittleEndian.AppendUint32(nil, protocol)
	b = binary.LittleEndian.AppendUint64(b, reqId)
	b = append(b, kind)
	return append(b, body...)
}

func registryReqFrame(kind uint8, body []byte) []byte {
	b := binary.LittleEndian.AppendUint32(nil, msgs.REGISTRY_REQ_PROTOCOL_VERSION)
	b = binary.LittleEndian.AppendUint32(b, uint32(1+len(body)))
	b = append(b, kind)
	return append(b, body...)
}

// registryRespFrame mirrors the server side read by readRegistryResponse: protocol, length
// of the data, data (kind byte first).
func registryRespFrame(kind uint8, body []byte) []byte {
	b := binary.LittleEndian.AppendUint32(nil, msgs.REGISTRY_RESP_PROTOCOL_VERSION)
	b = binary.LittleEndian.AppendUint32(b, uint32(1+len(body)))
	b = append(b, kind)
	return append(b, body...)
}

func blocksReqFrame(bsId msgs.BlockServiceId, kind uint8, body []byte) []byte {
	b := binary.LittleEndian.AppendUint32(nil, msgs.BLOCKS_REQ_PROTOCOL_VERSION)
	b = binary.LittleEndian.AppendUint64(b, uint64(bsId))
	b = append(b, kind)
	return append(b, body...)
}

func blocksRespFrame(kind uint8, body []byte) []byte {
	b := binary.LittleEndian.AppendUint32(nil, msgs.BLOCKS_RESP_PROTOCOL_VERSION)
	b = append(b, kind)
	return append(b, body...)
}

func u16(v uint16) []byte { return binary.LittleEndian.AppendUint16(nil, v) }

// pagesWithCrc lays out data the way block services serve it: each 4 KiB page followed
// by its little-endian CRC32C.
func pagesWithCrc(data []byte) []byte {
	var out []byte
	for off := 0; off < len(data); off += int(msgs.TERN_PAGE_SIZE) {
		page := data[off : off+int(msgs.TERN_PAGE_SIZE)]
		out = append(out, page...)
		out = binary.LittleEndian.AppendUint32(out, crc32c.Sum(0, page))
	}
	return out
}

func frames() []string {
	var lines []string
	add := func(kind string, b []byte, desc string) {
		lines = append(lines, fmt.Sprintf("%s %s # %s", kind, hexOrDash(b), desc))
	}
	lookup := pack(&msgs.LookupReq{DirId: msgs.ROOT_DIR_INODE_ID, Name: "hello.txt"})
	add("shard_req", shardFrame(msgs.SHARD_REQ_PROTOCOL_VERSION, 1, uint8(msgs.LOOKUP), lookup), "LOOKUP request id 1")
	add("shard_resp", shardFrame(msgs.SHARD_RESP_PROTOCOL_VERSION, 1, uint8(msgs.LOOKUP),
		pack(&msgs.LookupResp{TargetId: fileId(3, 99), CreationTime: 1})), "LOOKUP response id 1")
	add("shard_resp", shardFrame(msgs.SHARD_RESP_PROTOCOL_VERSION, 0xffff_ffff_ffff_ffff, msgs.ERROR, u16(uint16(msgs.NAME_NOT_FOUND))), "ERROR NAME_NOT_FOUND, max request id")
	add("shard_resp", shardFrame(msgs.SHARD_RESP_PROTOCOL_VERSION, 2, msgs.ERROR, u16(uint16(msgs.TIMEOUT))), "ERROR TIMEOUT")
	add("shard_resp", shardFrame(msgs.SHARD_RESP_PROTOCOL_VERSION, 3, uint8(msgs.READ_DIR), pack(maxReadDir())), "READ_DIR response at MTU")
	add("cdc_req", shardFrame(msgs.CDC_REQ_PROTOCOL_VERSION, 4, uint8(msgs.MAKE_DIRECTORY), []byte{}), "CDC MAKE_DIRECTORY header, empty body")
	add("cdc_resp", shardFrame(msgs.CDC_RESP_PROTOCOL_VERSION, 4, msgs.ERROR, u16(uint16(msgs.INTERNAL_ERROR))), "CDC ERROR INTERNAL_ERROR")
	add("registry_req", registryReqFrame(uint8(msgs.LOCAL_SHARDS), pack(&msgs.LocalShardsReq{})), "LOCAL_SHARDS request")
	shards := messages()["LocalShardsResp"][0].v
	add("registry_resp", registryRespFrame(uint8(msgs.LOCAL_SHARDS), pack(shards)), "LOCAL_SHARDS response, 256 shards")
	add("registry_resp", registryRespFrame(msgs.ERROR, u16(uint16(msgs.TIMEOUT))), "registry ERROR TIMEOUT")
	fetch := pack(&msgs.FetchBlockWithCrcReq{BlockId: 0xb10c, Offset: 0, Count: 2 * msgs.TERN_PAGE_SIZE})
	add("blocks_req", blocksReqFrame(0x1000_0000_0000, uint8(msgs.FETCH_BLOCK_WITH_CRC), fetch), "FETCH_BLOCK_WITH_CRC two pages")
	add("blocks_resp", blocksRespFrame(uint8(msgs.FETCH_BLOCK_WITH_CRC), pagesWithCrc(randBytes(2*int(msgs.TERN_PAGE_SIZE)))), "FETCH_BLOCK_WITH_CRC two pages with CRCs")
	add("blocks_resp", blocksRespFrame(msgs.ERROR, u16(uint16(msgs.BLOCK_NOT_FOUND))), "blocks ERROR BLOCK_NOT_FOUND")
	return lines
}

func crcCases() []string {
	var lines []string
	sum := func(data []byte, desc string) {
		lines = append(lines, fmt.Sprintf("sum %s %08x # %s", hexOrDash(data), crc32c.Sum(0, data), desc))
	}
	sum(nil, "empty")
	sum([]byte("123456789"), "check value")
	for _, n := range []int{1, 7, 4096, 4097} {
		sum(randBytes(n), fmt.Sprintf("%d random bytes", n))
	}
	for _, n := range []int{0, 1, 300, 4096} {
		a, b := randBytes(n+5), randBytes(n)
		ca, cb := crc32c.Sum(0, a), crc32c.Sum(0, b)
		lines = append(lines, fmt.Sprintf("append %08x %08x %d %08x # crc(a ‖ b), len(b)=%d", ca, cb, len(b), crc32c.Append(ca, cb, len(b)), n))
	}
	for _, n := range []int{0, 1, 4096, 1 << 20} {
		a := randBytes(33)
		ca := crc32c.Sum(0, a)
		lines = append(lines, fmt.Sprintf("zero_extend %08x %d %08x # crc(a ‖ 0^%d)", ca, n, crc32c.ZeroExtend(ca, n), n))
	}
	for _, n := range []int{4, 4096} {
		a, b := randBytes(n), randBytes(n)
		ca, cb := crc32c.Sum(0, a), crc32c.Sum(0, b)
		lines = append(lines, fmt.Sprintf("xor %08x %08x %d %08x # crc(a ^ b), len=%d", ca, cb, n, crc32c.Xor(ca, cb, n), n))
	}
	return lines
}

// msgsCRC is the upstream source named in the header of every golden file except rs.hex.
const msgsCRC = "go/msgs and go/core/crc32c"

func write(dir, file, from string, lines []string) {
	header := "# Generated by tools/goldens from upstream " + from + ". DO NOT EDIT.\n"
	body := header + strings.Join(lines, "\n") + "\n"
	if err := os.WriteFile(filepath.Join(dir, file), []byte(body), 0o644); err != nil {
		panic(err)
	}
}

func main() {
	if len(os.Args) != 2 {
		fmt.Fprintln(os.Stderr, "usage: goldens <output dir>")
		os.Exit(2)
	}
	dir := os.Args[1]
	if err := os.MkdirAll(dir, 0o755); err != nil {
		panic(err)
	}
	// Order matters for determinism: the shared rng is consumed in a fixed order.
	all := messages()
	names := make([]string, 0, len(all))
	for n := range all {
		names = append(names, n)
	}
	sort.Strings(names)
	for _, n := range names {
		var lines []string
		for _, s := range all[n] {
			lines = append(lines, fmt.Sprintf("%s # %s", hexOrDash(pack(s.v)), s.desc))
		}
		write(dir, n+".hex", msgsCRC, lines)
	}
	write(dir, "messages.hex", msgsCRC, generated())
	write(dir, "names.txt", msgsCRC, upstreamNames())
	write(dir, "frames.hex", msgsCRC, frames())
	write(dir, "crc32c.hex", msgsCRC, crcCases())
	write(dir, "rs.hex", "go/core/rs", rsCases())
}

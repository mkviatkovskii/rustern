// Copyright 2026 Mikhail Kviatkovskii
// Adapted from TernFS go/bincodegen/bincodegen.go (upstream 5550891) by XTX Markets
// Technologies Limited; see NOTICE.
//
// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

// rsgen emits crates/rustern-msgs/src/generated.rs: Rust structs and straight-line
// Encode/Decode impls for the TernFS messages, by reflecting over upstream go/msgs the
// same way go/bincodegen does (research R3). Adapted from go/bincodegen/bincodegen.go.
//
// Usage (from tools/): go run ./rsgen > ../crates/rustern-msgs/src/generated.rs
package main

import (
	"bytes"
	"fmt"
	"os"
	"os/exec"
	"reflect"
	"regexp"
	"sort"
	"strings"

	"github.com/XTXMarkets/ternfs/go/msgs"

	"rustern.local/tools/msglist"
)

// Upstream encodes these two unions by hand (go/msgs/msgs_bincode.go); so does
// crates/rustern-msgs/src/spans.rs.
var handWritten = map[string]bool{"FetchedSpan": true, "FetchedFullSpan": true}

// Fields whose Debug output must not show their bytes (FR-017).
var secretFields = map[string]bool{
	"Cookie": true, "Cookie1": true, "Cookie2": true,
	"Certificate": true, "Proof": true, "SecretKey": true,
}

// Requests that do not change cluster state (cpp/shard/ShardDB.cpp readOnlyShardReq and
// the read-only registry/blocks requests the PoC sends).
var readOnly = []string{
	"LookupReq", "StatFileReq", "StatDirectoryReq", "ReadDirReq", "LocalFileSpansReq",
	"FileSpansReq", "LocalShardsReq", "FetchBlockWithCrcReq",
}

const ternErrorCodeOffset = 10

// Imports from crate::wire that only some outputs need.
var usedImports = map[string]bool{}

type service struct {
	name   string // Rust prefix: Shard, Cdc, Registry, Blocks
	kind   string // Go kind type name
	reqRes []msglist.ReqResp
}

type gen struct {
	out      bytes.Buffer
	structs  []reflect.Type
	seen     map[reflect.Type]bool
	newtypes []reflect.Type
	seenNT   map[reflect.Type]bool
}

func (g *gen) p(format string, args ...any) {
	fmt.Fprintf(&g.out, format, args...)
	g.out.WriteByte('\n')
}

func isBlob(t reflect.Type) bool { return t.Kind() == reflect.Slice && t.Name() == "Blob" }

func isNamedScalar(t reflect.Type) bool {
	if t.PkgPath() == "" || t.Name() == "" {
		return false
	}
	switch t.Kind() {
	case reflect.Bool, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64, reflect.Array:
		return true
	}
	return false
}

func (g *gen) visit(t reflect.Type) {
	switch t.Kind() {
	case reflect.Struct:
		if handWritten[t.Name()] || g.seen[t] {
			return
		}
		g.seen[t] = true
		for i := 0; i < t.NumField(); i++ {
			g.visit(t.Field(i).Type)
		}
		g.structs = append(g.structs, t)
	case reflect.Slice:
		if !isBlob(t) {
			g.visit(t.Elem())
		}
	case reflect.Array:
		if t.Elem().Kind() != reflect.Uint8 {
			panic(fmt.Sprintf("only byte arrays are supported, got %v", t))
		}
		fallthrough
	case reflect.Bool, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64:
		if isNamedScalar(t) && !g.seenNT[t] {
			g.seenNT[t] = true
			g.newtypes = append(g.newtypes, t)
		}
	case reflect.String:
	default:
		panic(fmt.Sprintf("unsupported type %v (kind %v)", t, t.Kind()))
	}
}

func isLen32(t reflect.Type, tag string) bool {
	switch tag {
	case "":
		return false
	case "len32":
		if t.Kind() != reflect.Slice {
			panic(fmt.Sprintf("len32 on non-list %v", t))
		}
		return true
	}
	panic(fmt.Sprintf("unknown bincode tag %q", tag))
}

func parseTag(tag reflect.StructTag) string {
	if tag == "" {
		return ""
	}
	m := regexp.MustCompile(`^bincode:"([a-z0-9]+)"$`).FindStringSubmatch(string(tag))
	if m == nil {
		panic(fmt.Sprintf("invalid tag %v", tag))
	}
	return m[1]
}

func scalarRust(k reflect.Kind) string {
	switch k {
	case reflect.Bool:
		return "bool"
	case reflect.Uint8:
		return "u8"
	case reflect.Uint16:
		return "u16"
	case reflect.Uint32:
		return "u32"
	case reflect.Uint64:
		return "u64"
	}
	panic(k)
}

func rustType(t reflect.Type, tag string) string {
	switch t.Kind() {
	case reflect.Bool, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64:
		if isNamedScalar(t) {
			return t.Name()
		}
		return scalarRust(t.Kind())
	case reflect.Array:
		if isNamedScalar(t) {
			return t.Name()
		}
		return fmt.Sprintf("[u8; %d]", t.Len())
	case reflect.String:
		return "Bytes"
	case reflect.Slice:
		if isBlob(t) {
			if isLen32(t, tag) {
				usedImports["Blob32"] = true
				return "Blob32"
			}
			usedImports["Blob"] = true
			return "Blob"
		}
		if t.Elem().Kind() == reflect.Uint8 {
			return "Bytes"
		}
		return "Vec<" + rustType(t.Elem(), "") + ">"
	case reflect.Struct:
		if handWritten[t.Name()] {
			return "crate::spans::" + t.Name()
		}
		return t.Name()
	}
	panic(t)
}

// minSize is the smallest encoding of a value of t (research R3: list allocations are
// bounded by count x minSize).
func minSize(t reflect.Type, tag string) int {
	switch t.Kind() {
	case reflect.Bool, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64:
		return int(t.Size())
	case reflect.Array:
		return t.Len()
	case reflect.String:
		return 1
	case reflect.Slice:
		if t.Elem().Kind() == reflect.Uint8 && !isBlob(t) {
			return 1
		}
		if isLen32(t, tag) {
			return 4
		}
		return 2
	case reflect.Struct:
		switch t.Name() {
		case "FetchedSpan":
			return minSize(reflect.TypeOf(msgs.FetchedSpanHeader{}), "") +
				min(minSize(reflect.TypeOf(msgs.FetchedInlineSpan{}), ""), minSize(reflect.TypeOf(msgs.FetchedBlocksSpan{}), ""))
		case "FetchedFullSpan":
			return minSize(reflect.TypeOf(msgs.FetchedSpanHeaderFull{}), "") +
				min(minSize(reflect.TypeOf(msgs.FetchedInlineSpan{}), ""), minSize(reflect.TypeOf(msgs.FetchedLocations{}), ""))
		}
		n := 0
		for i := 0; i < t.NumField(); i++ {
			f := t.Field(i)
			n += minSize(f.Type, parseTag(f.Tag))
		}
		return n
	}
	panic(t)
}

var rustKeywords = map[string]bool{"type": true, "ref": true, "loop": true, "match": true, "move": true, "self": true, "fn": true, "mod": true, "use": true, "impl": true, "in": true, "as": true}

func snake(s string) string {
	var b strings.Builder
	r := []rune(s)
	for i, c := range r {
		up := c >= 'A' && c <= 'Z'
		if up && i > 0 {
			prev := r[i-1]
			prevLower := (prev >= 'a' && prev <= 'z') || (prev >= '0' && prev <= '9')
			nextLower := i+1 < len(r) && r[i+1] >= 'a' && r[i+1] <= 'z'
			if prevLower || (prev >= 'A' && prev <= 'Z' && nextLower) {
				b.WriteByte('_')
			}
		}
		if up {
			c = c - 'A' + 'a'
		}
		b.WriteRune(c)
	}
	out := b.String()
	if rustKeywords[out] {
		return "r#" + out
	}
	return out
}

// enumName matches go/bincodegen's enum naming (LOOKUP, STAT_FILE, ...).
func enumName(t reflect.Type) string {
	n := strings.TrimSuffix(strings.TrimSuffix(t.Name(), "Req"), "Resp")
	re := regexp.MustCompile(`(.)([A-Z])`)
	return strings.ToUpper(string(re.ReplaceAll([]byte(n), []byte("${1}_${2}"))))
}

func variantName(t reflect.Type) string { return strings.TrimSuffix(t.Name(), "Req") }

func (g *gen) newtype(t reflect.Type) {
	var inner string
	if t.Kind() == reflect.Array {
		inner = fmt.Sprintf("[u8; %d]", t.Len())
	} else {
		inner = scalarRust(t.Kind())
	}
	g.p("/// Upstream `%s.%s`.", t.PkgPath()[strings.LastIndex(t.PkgPath(), "/")+1:], t.Name())
	g.p("#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]")
	g.p("pub struct %s(pub %s);", t.Name(), inner)
	g.p("impl Encode for %s {", t.Name())
	g.p("fn encoded_len(&self) -> usize { self.0.encoded_len() }")
	g.p("fn encode(&self, out: &mut impl BufMut) -> Result<(), EncodeError> { self.0.encode(out) }")
	g.p("}")
	g.p("impl Decode for %s {", t.Name())
	g.p("const MIN_ENCODED_SIZE: usize = %d;", minSize(t, ""))
	g.p("fn decode(buf: &mut Bytes) -> Result<Self, DecodeError> { Decode::decode(buf).map(Self) }")
	g.p("}")
}

func (g *gen) structDef(t reflect.Type) {
	g.p("/// Upstream `msgs.%s`.", t.Name())
	g.p("#[derive(Clone, Default, PartialEq, Eq, Debug)]")
	if t.NumField() == 0 {
		g.p("pub struct %s {}", t.Name())
	} else {
		g.p("pub struct %s {", t.Name())
		for i := 0; i < t.NumField(); i++ {
			f := t.Field(i)
			ty := rustType(f.Type, parseTag(f.Tag))
			if secretFields[f.Name] {
				usedImports["Secret"] = true
				ty = "Secret<" + ty + ">"
			}
			g.p("pub %s: %s,", snake(f.Name), ty)
		}
		g.p("}")
	}
	out, buf := "out", "buf"
	if t.NumField() == 0 {
		out, buf = "_out", "_buf"
	}
	g.p("impl Encode for %s {", t.Name())
	g.p("fn encoded_len(&self) -> usize {")
	g.out.WriteString("0usize")
	for i := 0; i < t.NumField(); i++ {
		f := t.Field(i)
		if isLen32(f.Type, parseTag(f.Tag)) && !isBlob(f.Type) {
			fmt.Fprintf(&g.out, ".saturating_add(crate::wire::list32_encoded_len(&self.%s))", snake(f.Name))
		} else {
			fmt.Fprintf(&g.out, ".saturating_add(self.%s.encoded_len())", snake(f.Name))
		}
	}
	g.p("\n}")
	g.p("fn encode(&self, %s: &mut impl BufMut) -> Result<(), EncodeError> {", out)
	for i := 0; i < t.NumField(); i++ {
		f := t.Field(i)
		if isLen32(f.Type, parseTag(f.Tag)) && !isBlob(f.Type) {
			g.p("crate::wire::encode_list32(&self.%s, out)?;", snake(f.Name))
		} else {
			g.p("self.%s.encode(out)?;", snake(f.Name))
		}
	}
	g.p("Ok(())")
	g.p("}")
	g.p("}")
	g.p("impl Decode for %s {", t.Name())
	g.p("const MIN_ENCODED_SIZE: usize = %d;", minSize(t, ""))
	g.p("fn decode(%s: &mut Bytes) -> Result<Self, DecodeError> {", buf)
	g.p("Ok(Self {")
	for i := 0; i < t.NumField(); i++ {
		f := t.Field(i)
		if isLen32(f.Type, parseTag(f.Tag)) && !isBlob(f.Type) {
			g.p("%s: crate::wire::decode_list32(buf)?,", snake(f.Name))
		} else {
			g.p("%s: Decode::decode(buf)?,", snake(f.Name))
		}
	}
	g.p("})")
	g.p("}")
	g.p("}")
}

func (g *gen) kinds(s service) {
	rrs := append([]msglist.ReqResp(nil), s.reqRes...)
	sort.SliceStable(rrs, func(i, j int) bool { return rrs[i].Kind < rrs[j].Kind })
	g.p("/// Upstream `msgs.%s`.", s.kind)
	g.p("#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]")
	g.p("#[repr(u8)]")
	g.p("pub enum %sMessageKind {", s.name)
	for _, rr := range rrs {
		if enumName(rr.Req) != enumName(rr.Resp) {
			panic(fmt.Sprintf("bad pair %v %v", rr.Req, rr.Resp))
		}
		g.p("%s = 0x%02X,", variantName(rr.Req), rr.Kind)
	}
	g.p("}")
	g.p("impl %sMessageKind {", s.name)
	g.p("/// Every kind, in wire-value order.")
	g.p("pub const ALL: [Self; %d] = [", len(rrs))
	for _, rr := range rrs {
		g.p("Self::%s,", variantName(rr.Req))
	}
	g.p("];")
	g.p("/// Upstream name, as used in logs and metric labels.")
	g.p("pub const fn as_str(self) -> &'static str {")
	g.p("match self {")
	for _, rr := range rrs {
		g.p("Self::%s => %q,", variantName(rr.Req), enumName(rr.Req))
	}
	g.p("}")
	g.p("}")
	g.p("}")
	g.p("impl TryFrom<u8> for %sMessageKind {", s.name)
	g.p("type Error = u8;")
	g.p("fn try_from(v: u8) -> Result<Self, u8> {")
	g.p("match v {")
	for _, rr := range rrs {
		g.p("0x%02X => Ok(Self::%s),", rr.Kind, variantName(rr.Req))
	}
	g.p("other => Err(other),")
	g.p("}")
	g.p("}")
	g.p("}")
	g.p("/// A request to the %s service.", strings.ToLower(s.name))
	g.p("pub trait %sRequest: Encode + Decode {", s.name)
	g.p("/// Message kind on the wire.")
	g.p("const KIND: %sMessageKind;", s.name)
	g.p("/// The matching response type.")
	g.p("type Response: %sResponse;", s.name)
	g.p("}")
	g.p("/// A response from the %s service.", strings.ToLower(s.name))
	g.p("pub trait %sResponse: Encode + Decode {", s.name)
	g.p("/// Message kind on the wire.")
	g.p("const KIND: %sMessageKind;", s.name)
	g.p("}")
	for _, rr := range s.reqRes {
		v := variantName(rr.Req)
		g.p("impl %sRequest for %s { const KIND: %sMessageKind = %sMessageKind::%s; type Response = %s; }",
			s.name, rr.Req.Name(), s.name, s.name, v, rr.Resp.Name())
		g.p("impl %sResponse for %s { const KIND: %sMessageKind = %sMessageKind::%s; }",
			s.name, rr.Resp.Name(), s.name, s.name, v)
	}
}

// oversized returns a Rust expression for a value of t one past the maximum of its length
// prefix, or "" when t has no u8/u16 length prefix inside (u32 ones would need 4 GiB).
func (g *gen) oversized(t reflect.Type, tag string) string {
	switch t.Kind() {
	case reflect.String:
		return "bytes(256)"
	case reflect.Slice:
		switch {
		case isLen32(t, tag):
			return ""
		case isBlob(t):
			return "Blob(bytes(65536))"
		case t.Elem().Kind() == reflect.Uint8:
			return "bytes(256)"
		}
		elem := "Default::default()"
		switch t.Elem().Name() {
		case "FetchedSpan":
			elem = "span()"
		case "FetchedFullSpan":
			elem = "full_span()"
		}
		return fmt.Sprintf("vec![%s; 65536]", elem)
	case reflect.Struct:
		if !handWritten[t.Name()] && len(g.oversizedFields(t)) > 0 {
			return "big_" + strings.TrimPrefix(snake(t.Name()), "r#") + "()"
		}
	}
	return ""
}

// oversizedFields returns `field: expr` for every field of t that oversized can overflow.
func (g *gen) oversizedFields(t reflect.Type) []string {
	var out []string
	for i := 0; i < t.NumField(); i++ {
		f := t.Field(i)
		e := g.oversized(f.Type, parseTag(f.Tag))
		if e == "" {
			continue
		}
		if secretFields[f.Name] {
			e = "Secret(" + e + ")"
		}
		out = append(out, snake(f.Name)+": "+e)
	}
	return out
}

// oversizedTests emits a unit test that sets each length-prefixed field, one at a time,
// one past its prefix's maximum and checks that encoding fails.
func (g *gen) oversizedTests() {
	lit := func(t reflect.Type, field string) string {
		if t.NumField() == 1 {
			return fmt.Sprintf("%s { %s }", t.Name(), field)
		}
		return fmt.Sprintf("%s { %s, ..Default::default() }", t.Name(), field)
	}
	var helpers, cases []string
	for _, t := range g.structs {
		fields := g.oversizedFields(t)
		if len(fields) == 0 {
			continue
		}
		helpers = append(helpers, fmt.Sprintf("fn %s -> %s { %s }", g.oversized(t, ""), t.Name(), lit(t, fields[0])))
		for _, f := range fields {
			cases = append(cases, fmt.Sprintf("fails(&%s);", lit(t, f)))
		}
	}
	body := strings.Join(cases, "\n")
	g.p("")
	g.p("#[cfg(test)]")
	g.p("mod oversized {")
	g.p("//! Every u8/u16 length-prefixed field, one at a time, one past the prefix's maximum:")
	g.p("//! encoding fails instead of writing a truncated length.")
	g.p("use super::*;")
	if strings.Contains(body, "span()") {
		g.p("use crate::spans::{FetchedSpan, FetchedSpanBody};")
		g.p("fn span() -> FetchedSpan {")
		g.p("FetchedSpan { header: FetchedSpanHeader::default(), body: FetchedSpanBody::Inline(FetchedInlineSpan::default()) }")
		g.p("}")
	}
	if strings.Contains(body, "full_span()") {
		g.p("use crate::spans::{FetchedFullSpan, FetchedFullSpanBody};")
		g.p("fn full_span() -> FetchedFullSpan {")
		g.p("FetchedFullSpan { header: FetchedSpanHeaderFull::default(), body: FetchedFullSpanBody::Inline(FetchedInlineSpan::default()) }")
		g.p("}")
	}
	g.p("fn bytes(n: usize) -> Bytes { Bytes::from(vec![0; n]) }")
	g.p("fn fails<T: Encode>(v: &T) {")
	g.p("assert!(crate::wire::encode_to_vec(v).is_err());")
	g.p("}")
	// Only the helpers some case or emitted helper calls.
	used := body
	for changed := true; changed; {
		changed = false
		for _, h := range helpers {
			name := strings.Fields(h)[1]
			if strings.Contains(used, name) && !strings.Contains(used, h) {
				used += "\n" + h
				changed = true
			}
		}
	}
	for _, h := range helpers {
		if strings.Contains(used, h) {
			g.p("%s", h)
		}
	}
	g.p("#[test]")
	g.p("fn oversized_fields_fail_to_encode() {")
	g.p("%s", body)
	g.p("}")
	g.p("}")
}

func main() {
	l := msglist.Upstream()
	g := &gen{seen: map[reflect.Type]bool{}, seenNT: map[reflect.Type]bool{}}
	services := []service{
		{"Shard", "ShardMessageKind", l.Shard},
		{"Cdc", "CDCMessageKind", l.Cdc},
		{"Registry", "RegistryMessageKind", l.Registry},
		{"Blocks", "BlocksMessageKind", l.Blocks},
	}
	for _, s := range services {
		for _, rr := range s.reqRes {
			g.visit(rr.Req)
			g.visit(rr.Resp)
		}
	}
	for _, t := range l.Extras {
		g.visit(t)
	}
	// Parts of the hand-written unions.
	for _, t := range []reflect.Type{
		reflect.TypeOf(msgs.FetchedSpanHeader{}), reflect.TypeOf(msgs.FetchedInlineSpan{}),
		reflect.TypeOf(msgs.FetchedBlocksSpan{}), reflect.TypeOf(msgs.FetchedSpanHeaderFull{}),
		reflect.TypeOf(msgs.FetchedLocations{}),
	} {
		g.visit(t)
	}
	sort.Slice(g.newtypes, func(i, j int) bool { return g.newtypes[i].Name() < g.newtypes[j].Name() })

	g.p("// Copyright 2026 Mikhail Kviatkovskii")
	g.p("// Generated from TernFS go/msgs by XTX Markets Technologies Limited; see NOTICE.")
	g.p("//")
	g.p("// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception")
	g.p("")
	g.p("// Generated by tools/rsgen from upstream go/msgs. DO NOT EDIT.")
	g.p("")
	g.p("//! TernFS messages generated from upstream `go/msgs`.")
	g.p("#![allow(non_camel_case_types, missing_docs)]")
	g.p("")
	g.p("use bytes::{BufMut, Bytes};")
	g.p("")
	var secretTypes []string
	for _, t := range g.structs {
		ty := t
		for i := 0; i < ty.NumField(); i++ {
			f := ty.Field(i)
			rustType(f.Type, parseTag(f.Tag))
			if secretFields[f.Name] {
				usedImports["Secret"] = true
				if len(secretTypes) == 0 || secretTypes[len(secretTypes)-1] != ty.Name() {
					secretTypes = append(secretTypes, ty.Name())
				}
			}
		}
	}
	imports := []string{"Decode", "DecodeError", "Encode", "EncodeError"}
	for _, n := range []string{"Blob", "Blob32", "Secret"} {
		if usedImports[n] {
			imports = append(imports, n)
		}
	}
	sort.Strings(imports)
	g.p("use crate::wire::{%s};", strings.Join(imports, ", "))
	g.p("")
	g.p("pub const SHARD_REQ_PROTOCOL_VERSION: u32 = 0x%X;", msgs.SHARD_REQ_PROTOCOL_VERSION)
	g.p("pub const SHARD_RESP_PROTOCOL_VERSION: u32 = 0x%X;", msgs.SHARD_RESP_PROTOCOL_VERSION)
	g.p("pub const CDC_REQ_PROTOCOL_VERSION: u32 = 0x%X;", msgs.CDC_REQ_PROTOCOL_VERSION)
	g.p("pub const CDC_RESP_PROTOCOL_VERSION: u32 = 0x%X;", msgs.CDC_RESP_PROTOCOL_VERSION)
	g.p("pub const REGISTRY_REQ_PROTOCOL_VERSION: u32 = 0x%X;", msgs.REGISTRY_REQ_PROTOCOL_VERSION)
	g.p("pub const REGISTRY_RESP_PROTOCOL_VERSION: u32 = 0x%X;", msgs.REGISTRY_RESP_PROTOCOL_VERSION)
	g.p("pub const BLOCKS_REQ_PROTOCOL_VERSION: u32 = 0x%X;", msgs.BLOCKS_REQ_PROTOCOL_VERSION)
	g.p("pub const BLOCKS_RESP_PROTOCOL_VERSION: u32 = 0x%X;", msgs.BLOCKS_RESP_PROTOCOL_VERSION)
	g.p("/// Message kind of an error response.")
	g.p("pub const ERROR_KIND: u8 = %d;", msgs.ERROR)
	g.p("pub const DEFAULT_UDP_MTU: usize = %d;", msgs.DEFAULT_UDP_MTU)
	g.p("pub const MAX_UDP_MTU: usize = %d;", msgs.MAX_UDP_MTU)
	g.p("pub const TERN_PAGE_SIZE: u32 = %d;", msgs.TERN_PAGE_SIZE)
	g.p("pub const TERN_PAGE_WITH_CRC_SIZE: u32 = %d;", msgs.TERN_PAGE_WITH_CRC_SIZE)
	g.p("pub const INODE_TYPE_DIRECTORY: u8 = %d;", msgs.DIRECTORY)
	g.p("pub const INODE_TYPE_FILE: u8 = %d;", msgs.FILE)
	g.p("pub const INODE_TYPE_SYMLINK: u8 = %d;", msgs.SYMLINK)
	g.p("pub const ROOT_DIR_INODE_ID: InodeId = InodeId(0x%X);", uint64(msgs.ROOT_DIR_INODE_ID))
	g.p("pub const NULL_INODE_ID: InodeId = InodeId(0x%X);", uint64(msgs.NULL_INODE_ID))
	g.p("pub const EMPTY_STORAGE: StorageClass = StorageClass(%d);", msgs.EMPTY_STORAGE)
	g.p("pub const INLINE_STORAGE: StorageClass = StorageClass(%d);", msgs.INLINE_STORAGE)
	g.p("pub const HDD_STORAGE: StorageClass = StorageClass(%d);", msgs.HDD_STORAGE)
	g.p("pub const FLASH_STORAGE: StorageClass = StorageClass(%d);", msgs.FLASH_STORAGE)
	g.p("")

	// Error codes: a newtype, so codes unknown to this build still decode.
	g.p("/// Upstream `msgs.TernError`.")
	g.p("#[derive(Clone, Copy, PartialEq, Eq, Hash)]")
	g.p("pub struct ErrorCode(pub u16);")
	g.p("impl ErrorCode {")
	for i, e := range l.Errors {
		code := uint16(i + ternErrorCodeOffset)
		if msgs.TernError(code).String() != e {
			panic(fmt.Sprintf("error %s: upstream name %s", e, msgs.TernError(code).String()))
		}
		g.p("pub const %s: Self = Self(%d);", e, code)
	}
	g.p("/// Upstream name, or `None` for a code this build does not know.")
	g.p("pub const fn name(self) -> Option<&'static str> {")
	g.p("match self.0 {")
	for i, e := range l.Errors {
		g.p("%d => Some(%q),", i+ternErrorCodeOffset, e)
	}
	g.p("_ => None,")
	g.p("}")
	g.p("}")
	g.p("}")
	g.p("impl std::fmt::Debug for ErrorCode {")
	g.p("fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {")
	g.p("match self.name() { Some(n) => f.write_str(n), None => write!(f, \"ErrorCode({})\", self.0) }")
	g.p("}")
	g.p("}")
	g.p("impl std::fmt::Display for ErrorCode {")
	g.p("fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { std::fmt::Debug::fmt(self, f) }")
	g.p("}")
	g.p("impl Encode for ErrorCode {")
	g.p("fn encoded_len(&self) -> usize { 2 }")
	g.p("fn encode(&self, out: &mut impl BufMut) -> Result<(), EncodeError> { self.0.encode(out) }")
	g.p("}")
	g.p("impl Decode for ErrorCode {")
	g.p("const MIN_ENCODED_SIZE: usize = 2;")
	g.p("fn decode(buf: &mut Bytes) -> Result<Self, DecodeError> { Decode::decode(buf).map(Self) }")
	g.p("}")
	g.p("")

	for _, t := range g.newtypes {
		g.newtype(t)
	}
	for _, t := range g.structs {
		g.structDef(t)
	}
	for _, s := range services {
		g.kinds(s)
	}
	g.p("/// A request that never changes cluster state. Only these can be sent (FR-002).")
	g.p("pub trait ReadOnlyRequest {}")
	for _, n := range readOnly {
		g.p("impl ReadOnlyRequest for %s {}", n)
	}
	g.p("/// Names of the fields wrapped in `Secret` (FR-017).")
	names := make([]string, 0, len(secretFields))
	for n := range secretFields {
		names = append(names, fmt.Sprintf("%q", snake(n)))
	}
	sort.Strings(names)
	g.p("pub const SECRET_FIELDS: [&str; %d] = [%s];", len(names), strings.Join(names, ", "))
	g.p("/// Invokes `$m!(Type)` for every generated type with a `Secret` field (FR-017 tests).")
	g.p("#[doc(hidden)]")
	g.p("#[macro_export]")
	g.p("macro_rules! for_each_secret_type {")
	g.p("($m:ident) => {")
	for _, n := range secretTypes {
		g.p("$m!(%s);", n)
	}
	g.p("};")
	g.p("}")
	messages := make([]string, 0, len(g.structs)+len(handWritten))
	for _, t := range g.structs {
		messages = append(messages, t.Name())
	}
	for n := range handWritten {
		messages = append(messages, n)
	}
	sort.Strings(messages)
	g.p("/// Invokes `$m!(Type, ...)` with every message struct, generated and hand-written")
	g.p("/// (golden tests).")
	g.p("#[doc(hidden)]")
	g.p("#[macro_export]")
	g.p("macro_rules! for_each_message {")
	g.p("($m:ident) => {")
	g.p("$m!(%s);", strings.Join(messages, ", "))
	g.p("};")
	g.p("}")
	g.oversizedTests()

	cmd := exec.Command("rustfmt", "--edition", "2024", "--emit", "stdout", "--quiet")
	cmd.Stdin = &g.out
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		fmt.Fprintf(os.Stderr, "rustfmt failed: %v\n", err)
		os.Exit(1)
	}
}

package main

import (
	"fmt"
	"math/rand"
	"reflect"

	"github.com/XTXMarkets/ternfs/go/core/bincode"
	"github.com/XTXMarkets/ternfs/go/msgs"

	"rustern.local/tools/msglist"
)

// filler sets every field of a message by reflection: all zero, random, or maximal
// (all-ones scalars, 255-byte strings, 300-byte blobs, three-element lists).
type filler struct {
	rng    *rand.Rand
	mode   string // "zero", "random" or "max"
	unions int    // alternates the union bodies
}

func (f *filler) length(longest, random int) int {
	switch f.mode {
	case "zero":
		return 0
	case "max":
		return longest
	}
	return f.rng.Intn(random + 1)
}

func (f *filler) bytes(n int) []byte {
	b := make([]byte, n)
	for i := range b {
		if f.mode == "max" {
			b[i] = 0xff
		} else {
			b[i] = byte(f.rng.Intn(256))
		}
	}
	return b
}

func (f *filler) fill(v reflect.Value) {
	t := v.Type()
	switch t.Kind() {
	case reflect.Bool:
		v.SetBool(f.mode == "max" || (f.mode == "random" && f.rng.Intn(2) == 1))
	case reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64:
		switch f.mode {
		case "max":
			v.SetUint(^uint64(0) >> (64 - 8*t.Size()))
		case "random":
			v.SetUint(f.rng.Uint64() >> (64 - 8*t.Size()))
		}
	case reflect.Array:
		if f.mode != "zero" {
			reflect.Copy(v, reflect.ValueOf(f.bytes(t.Len())))
		}
	case reflect.String:
		v.SetString(string(f.bytes(f.length(255, 8))))
	case reflect.Slice:
		if t.Elem().Kind() == reflect.Uint8 {
			longest := 255 // u8 length prefix
			if t.Name() == "Blob" {
				longest = 300 // u16 or u32 length prefix
			}
			v.Set(reflect.ValueOf(f.bytes(f.length(longest, 8))).Convert(t))
			return
		}
		n := f.length(3, 2)
		s := reflect.MakeSlice(t, n, n)
		for i := 0; i < n; i++ {
			f.fill(s.Index(i))
		}
		v.Set(s)
	case reflect.Struct:
		if f.union(v) {
			return
		}
		for i := 0; i < t.NumField(); i++ {
			f.fill(v.Field(i))
		}
	default:
		panic(fmt.Sprintf("cannot fill %v", t))
	}
}

// union fills the hand-encoded unions, alternating their two bodies; the header says
// which body follows.
func (f *filler) union(v reflect.Value) bool {
	switch p := v.Addr().Interface().(type) {
	case *msgs.FetchedSpan:
		f.fill(reflect.ValueOf(&p.Header).Elem())
		f.unions++
		if f.mode == "zero" || f.unions%2 == 1 {
			p.Header.StorageClass = msgs.INLINE_STORAGE
			b := &msgs.FetchedInlineSpan{}
			f.fill(reflect.ValueOf(b).Elem())
			p.Body = b
		} else {
			if p.Header.StorageClass == msgs.EMPTY_STORAGE || p.Header.StorageClass == msgs.INLINE_STORAGE {
				p.Header.StorageClass = msgs.HDD_STORAGE
			}
			b := &msgs.FetchedBlocksSpan{}
			f.fill(reflect.ValueOf(b).Elem())
			p.Body = b
		}
		return true
	case *msgs.FetchedFullSpan:
		f.fill(reflect.ValueOf(&p.Header).Elem())
		f.unions++
		p.Header.IsInline = f.mode == "zero" || f.unions%2 == 1
		if p.Header.IsInline {
			b := &msgs.FetchedInlineSpan{}
			f.fill(reflect.ValueOf(b).Elem())
			p.Body = b
		} else {
			b := &msgs.FetchedLocations{}
			f.fill(reflect.ValueOf(b).Elem())
			p.Body = b
		}
		return true
	}
	return false
}

// generated returns `<Type> <hex> # <desc>` lines: for every struct of every upstream
// message, a zero, two random and a maximal sample (duplicates dropped), so every
// generated codec is checked against Go.
func generated() []string {
	f := &filler{rng: rand.New(rand.NewSource(0x6e6))}
	var lines []string
	for _, t := range msglist.Structs(msglist.Upstream()) {
		seen := map[string]bool{}
		for _, mode := range []string{"zero", "random", "random", "max"} {
			f.mode = mode
			v := reflect.New(t)
			f.fill(v.Elem())
			p, ok := v.Interface().(bincode.Packable)
			if !ok {
				panic(fmt.Sprintf("%v is not Packable", t))
			}
			h := hexOrDash(pack(p))
			if seen[h] {
				continue
			}
			seen[h] = true
			lines = append(lines, fmt.Sprintf("%s %s # %s", t.Name(), h, mode))
		}
	}
	return lines
}

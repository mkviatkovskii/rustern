// Copyright 2026 Mikhail Kviatkovskii
//
// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

package rsbench

import (
	"fmt"
	"testing"

	"github.com/XTXMarkets/ternfs/go/core/parity"
	"github.com/XTXMarkets/ternfs/go/core/rs"
)

const block = 1 << 20

var layouts = [][2]int{{4, 2}, {10, 4}, {8, 3}}

func blocks(n int) [][]byte {
	out := make([][]byte, n)
	for b := range out {
		out[b] = make([]byte, block)
		for i := range out[b] {
			out[b][i] = byte((i * 31) ^ (b * 97))
		}
	}
	return out
}

func eachLevel(b *testing.B, f func(b *testing.B)) {
	for _, l := range []Level{Scalar, Avx2, Gfni} {
		if !l.Available() {
			continue
		}
		b.Run(l.String(), func(b *testing.B) {
			l.Use()
			f(b)
		})
	}
	Gfni.Use()
	if !Gfni.Available() {
		Avx2.Use()
	}
}

func BenchmarkParity(b *testing.B) {
	for _, dp := range layouts {
		d, p := dp[0], dp[1]
		r := rs.Get(parity.MkParity(uint8(d), uint8(p)))
		data := blocks(d)
		par := blocks(p)
		b.Run(fmt.Sprintf("D%dP%d", d, p), func(b *testing.B) {
			eachLevel(b, func(b *testing.B) {
				b.SetBytes(int64(d * block))
				for b.Loop() {
					r.ComputeParityInto(data, par)
				}
			})
		})
	}
}

// Recover data block 0 without parity block 0 (the XOR row), so the multiply kernel runs:
// have = data blocks 1..D-1 and parity block 1.
func BenchmarkRecover(b *testing.B) {
	for _, dp := range layouts {
		d, p := dp[0], dp[1]
		r := rs.Get(parity.MkParity(uint8(d), uint8(p)))
		data := blocks(d)
		par := r.ComputeParity(data)
		have := append(append([][]byte{}, data[1:]...), par[1])
		mask := uint32(1)<<d - 2 | uint32(1)<<(d+1)
		out := make([]byte, block)
		b.Run(fmt.Sprintf("D%dP%d", d, p), func(b *testing.B) {
			eachLevel(b, func(b *testing.B) {
				b.SetBytes(int64(d * block))
				for b.Loop() {
					r.RecoverInto(mask, have, 0, out)
				}
			})
		})
	}
}

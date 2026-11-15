package main

import (
	"encoding/hex"
	"fmt"
	"math/bits"
	"math/rand"

	"github.com/XTXMarkets/ternfs/go/core/parity"
	"github.com/XTXMarkets/ternfs/go/core/rs"
)

// rsRng is separate from rng so that adding RS cases never changes the other goldens.
var rsRng = rand.New(rand.NewSource(0x5eed_0f_25))

func rsBlocks(n, size int) [][]byte {
	blocks := make([][]byte, n)
	for i := range blocks {
		blocks[i] = make([]byte, size)
		rsRng.Read(blocks[i])
	}
	return blocks
}

func concatHex(blocks [][]byte) string {
	var out []byte
	for _, b := range blocks {
		out = append(out, b...)
	}
	return hex.EncodeToString(out)
}

// rsCases writes rs.hex: parity computed by upstream go/core/rs (the cgo build of rs.cpp,
// whichever CPU level it picks), and blocks recovered by it.
//
//	parity <D> <P> <data hex> <parity hex> # desc      (blocks concatenated, block size = len/D)
//	recover <D> <P> <have mask> <want> <have hex> <want hex> # desc
func rsCases() []string {
	var lines []string
	add := func(d, p, size int, desc string) {
		r := rs.Get(parity.MkParity(uint8(d), uint8(p)))
		data := rsBlocks(d, size)
		par := r.ComputeParity(data)
		lines = append(lines, fmt.Sprintf("parity %d %d %s %s # %s", d, p, concatHex(data), concatHex(par), desc))

		// Recover a random block from a random set of D others.
		all := append(append([][]byte{}, data...), par...)
		b := d + p
		perm := rsRng.Perm(b)
		want := perm[0]
		mask := uint32(0)
		for _, ix := range perm[1 : 1+d] {
			mask |= 1 << ix
		}
		have := make([][]byte, 0, d)
		for ix := 0; ix < b; ix++ {
			if mask&(1<<ix) != 0 {
				have = append(have, all[ix])
			}
		}
		if bits.OnesCount32(mask) != d {
			panic("bad mask")
		}
		got := r.Recover(mask, have, uint8(want))
		lines = append(lines, fmt.Sprintf("recover %d %d %08x %d %s %s # %s", d, p, mask, want, concatHex(have), hex.EncodeToString(got), desc))
	}
	// Every layout TernFS allows (D 2..10, P 1..6): two 32-byte chunks plus a 3-byte tail.
	for d := 2; d <= 10; d++ {
		for p := 1; p <= 6; p++ {
			add(d, p, 67, fmt.Sprintf("D=%d P=%d", d, p))
		}
	}
	// Block sizes around the 32-byte vector width.
	for _, size := range []int{1, 2, 15, 31, 32, 33, 63, 64, 65, 100, 4096 + 7} {
		add(4, 2, size, fmt.Sprintf("D=4 P=2 size=%d", size))
	}
	// Beyond the TernFS limits, up to what the parity byte encodes.
	for _, dp := range [][2]int{{15, 1}, {2, 15}, {15, 15}, {11, 7}, {12, 4}} {
		add(dp[0], dp[1], 97, fmt.Sprintf("D=%d P=%d", dp[0], dp[1]))
	}
	return lines
}

// Copyright 2026 Mikhail Kviatkovskii
//
// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

// Package rsbench benchmarks upstream go/core/rs per kernel, with the same layouts, block size,
// data and recovery case as crates/rustern-rs/benches/rs.rs, so the numbers compare directly:
//
//	go test -run '^$' -bench . ./rsbench
//
// Throughput (MB/s) is data bytes per second, D blocks of 1 MiB.
package rsbench

// // Defined in go/core/rs (rs.cpp) but not exported to Go.
// extern _Bool rs_has_cpu_level(int level);
// extern void rs_set_cpu_level(int level);
import "C"

// Level is upstream's enum rs_cpu_level.
type Level int

const (
	Scalar Level = 1
	Avx2   Level = 2
	Gfni   Level = 3
)

func (l Level) String() string {
	return [...]string{"?", "Scalar", "Avx2", "Gfni"}[l]
}

// Available reports whether this CPU supports the kernel.
func (l Level) Available() bool { return bool(C.rs_has_cpu_level(C.int(l))) }

// Use makes upstream run this kernel for every later call (process-wide).
func (l Level) Use() { C.rs_set_cpu_level(C.int(l)) }

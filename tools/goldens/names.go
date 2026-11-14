// Copyright 2025 XTX Markets Technologies Limited
//
// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

package main

import (
	"fmt"

	"github.com/XTXMarkets/ternfs/go/msgs"

	"rustern.local/tools/msglist"
)

// upstreamNames returns `<set> <value> <NAME>` lines: every message kind of every service and
// every error code, named by upstream String().
func upstreamNames() []string {
	l := msglist.Upstream()
	var lines []string
	add := func(set string, v uint16, name fmt.Stringer) {
		lines = append(lines, fmt.Sprintf("%s %d %s", set, v, name))
	}
	for _, rr := range l.Shard {
		add("shard", uint16(rr.Kind), msgs.ShardMessageKind(rr.Kind))
	}
	for _, rr := range l.Cdc {
		add("cdc", uint16(rr.Kind), msgs.CDCMessageKind(rr.Kind))
	}
	for _, rr := range l.Registry {
		add("registry", uint16(rr.Kind), msgs.RegistryMessageKind(rr.Kind))
	}
	for _, rr := range l.Blocks {
		add("blocks", uint16(rr.Kind), msgs.BlocksMessageKind(rr.Kind))
	}
	for code := uint16(0); code < 1000; code++ {
		if e := msgs.TernError(code); e.String() != fmt.Sprintf("TernError(%d)", code) {
			add("error", code, e)
		}
	}
	return lines
}

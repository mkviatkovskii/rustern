package msglist

import (
	"reflect"

	"github.com/XTXMarkets/ternfs/go/msgs"
)

// Unions are the structs upstream encodes by hand (go/msgs/msgs_bincode.go), with the
// structs their encoding is made of.
var Unions = map[string][]reflect.Type{
	"FetchedSpan": {
		reflect.TypeOf(msgs.FetchedSpanHeader{}), reflect.TypeOf(msgs.FetchedInlineSpan{}),
		reflect.TypeOf(msgs.FetchedBlocksSpan{}),
	},
	"FetchedFullSpan": {
		reflect.TypeOf(msgs.FetchedSpanHeaderFull{}), reflect.TypeOf(msgs.FetchedInlineSpan{}),
		reflect.TypeOf(msgs.FetchedLocations{}),
	},
}

// Structs returns every struct type the lists reach, unions and their parts included,
// each once, in first-visit order.
func Structs(l Lists) []reflect.Type {
	var out []reflect.Type
	seen := map[reflect.Type]bool{}
	var visit func(t reflect.Type)
	visit = func(t reflect.Type) {
		switch t.Kind() {
		case reflect.Struct:
			if seen[t] {
				return
			}
			seen[t] = true
			out = append(out, t)
			if parts, ok := Unions[t.Name()]; ok {
				for _, p := range parts {
					visit(p)
				}
				return
			}
			for i := 0; i < t.NumField(); i++ {
				visit(t.Field(i).Type)
			}
		case reflect.Slice:
			visit(t.Elem())
		}
	}
	for _, rrs := range [][]ReqResp{l.Shard, l.Cdc, l.Registry, l.Blocks} {
		for _, rr := range rrs {
			visit(rr.Req)
			visit(rr.Resp)
		}
	}
	for _, t := range l.Extras {
		visit(t)
	}
	return out
}

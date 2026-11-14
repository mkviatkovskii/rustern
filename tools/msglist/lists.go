// Copyright 2026 Mikhail Kviatkovskii
// The lists are TernFS's (go/bincodegen/bincodegen.go, upstream 5550891), by XTX Markets
// Technologies Limited; see NOTICE.
//
// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

// Message lists copied verbatim from go/bincodegen/bincodegen.go main() (upstream 5550891).
// Refresh by re-running the extraction when the upstream pin moves.

// Package msglist lists the upstream TernFS messages that tools/rsgen generates Rust for
// and tools/goldens writes golden samples of.
package msglist

import (
	"reflect"

	"github.com/XTXMarkets/ternfs/go/msgs"
)

// ReqResp is one message kind of a service with its request and response types.
type ReqResp struct {
	Kind uint8
	Req  reflect.Type
	Resp reflect.Type
}

// Lists is everything go/bincodegen generates: error names, per-service messages and the
// extra standalone types.
type Lists struct {
	Errors   []string
	Shard    []ReqResp
	Cdc      []ReqResp
	Registry []ReqResp
	Blocks   []ReqResp
	Extras   []reflect.Type
}

// Upstream returns the lists of go/bincodegen main().
func Upstream() Lists {
	errors := []string{
		"INTERNAL_ERROR",
		"FATAL_ERROR",
		"TIMEOUT",
		"MALFORMED_REQUEST",
		"MALFORMED_RESPONSE",
		"NOT_AUTHORISED",
		"UNRECOGNIZED_REQUEST",
		"FILE_NOT_FOUND",
		"DIRECTORY_NOT_FOUND",
		"NAME_NOT_FOUND",
		"EDGE_NOT_FOUND",
		"EDGE_IS_LOCKED",
		"TYPE_IS_DIRECTORY",
		"TYPE_IS_NOT_DIRECTORY",
		"BAD_COOKIE",
		"INCONSISTENT_STORAGE_CLASS_PARITY",
		"LAST_SPAN_STATE_NOT_CLEAN",
		"COULD_NOT_PICK_BLOCK_SERVICES",
		"BAD_SPAN_BODY",
		"SPAN_NOT_FOUND",
		"BLOCK_SERVICE_NOT_FOUND",
		"CANNOT_CERTIFY_BLOCKLESS_SPAN",
		"BAD_NUMBER_OF_BLOCKS_PROOFS",
		"BAD_BLOCK_PROOF",
		"CANNOT_OVERRIDE_NAME",
		"NAME_IS_LOCKED",
		"MTIME_IS_TOO_RECENT",
		"MISMATCHING_TARGET",
		"MISMATCHING_OWNER",
		"MISMATCHING_CREATION_TIME",
		"DIRECTORY_NOT_EMPTY",
		"FILE_IS_TRANSIENT",
		"OLD_DIRECTORY_NOT_FOUND",
		"NEW_DIRECTORY_NOT_FOUND",
		"LOOP_IN_DIRECTORY_RENAME",
		"DIRECTORY_HAS_OWNER",
		"FILE_IS_NOT_TRANSIENT",
		"FILE_NOT_EMPTY",
		"CANNOT_REMOVE_ROOT_DIRECTORY",
		"FILE_EMPTY",
		"CANNOT_REMOVE_DIRTY_SPAN",
		"BAD_SHARD",
		"BAD_NAME",
		"MORE_RECENT_SNAPSHOT_EDGE",
		"MORE_RECENT_CURRENT_EDGE",
		"BAD_DIRECTORY_INFO",
		"DEADLINE_NOT_PASSED",
		"SAME_SOURCE_AND_DESTINATION",
		"SAME_DIRECTORIES",
		"SAME_SHARD",
		"BAD_PROTOCOL_VERSION",
		"BAD_CERTIFICATE",
		"BLOCK_TOO_RECENT_FOR_DELETION",
		"BLOCK_FETCH_OUT_OF_BOUNDS",
		"BAD_BLOCK_CRC",
		"BLOCK_TOO_BIG",
		"BLOCK_NOT_FOUND",
		"CANNOT_UNSET_DECOMMISSIONED",
		"CANNOT_REGISTER_DECOMMISSIONED_OR_STALE",
		"BLOCK_TOO_OLD_FOR_WRITE",
		// these two error below are heavily heuristic based. the first one means
		// "we got a IO error, but the whole disk is probably kaput", the second
		// means "we got a IO error, and it's probably some isolated bad sectors
		// in the file".
		"BLOCK_IO_ERROR_DEVICE",
		"BLOCK_IO_ERROR_FILE",
		"INVALID_REPLICA",
		"DIFFERENT_ADDRS_INFO",
		"LEADER_PREEMPTED",
		"LOG_ENTRY_MISSING",
		"LOG_ENTRY_TRIMMED",
		"LOG_ENTRY_UNRELEASED",
		"LOG_ENTRY_RELEASED",
		"AUTO_DECOMMISSION_FORBIDDEN",
		"INCONSISTENT_BLOCK_SERVICE_REGISTRATION",
		"SWAP_BLOCKS_INLINE_STORAGE",
		"SWAP_BLOCKS_MISMATCHING_SIZE",
		"SWAP_BLOCKS_MISMATCHING_STATE",
		"SWAP_BLOCKS_MISMATCHING_CRC",
		"SWAP_BLOCKS_DUPLICATE_BLOCK_SERVICE",
		"SWAP_SPANS_INLINE_STORAGE",
		"SWAP_SPANS_MISMATCHING_SIZE",
		"SWAP_SPANS_NOT_CLEAN",
		"SWAP_SPANS_MISMATCHING_CRC",
		"SWAP_SPANS_MISMATCHING_BLOCKS",
		"EDGE_NOT_OWNED",
		"CANNOT_CREATE_DB_SNAPSHOT",
		"BLOCK_SIZE_NOT_MULTIPLE_OF_PAGE_SIZE",
		"SWAP_BLOCKS_DUPLICATE_FAILURE_DOMAIN",
		"TRANSIENT_LOCATION_COUNT",
		"ADD_SPAN_LOCATION_INLINE_STORAGE",
		"ADD_SPAN_LOCATION_MISMATCHING_SIZE",
		"ADD_SPAN_LOCATION_NOT_CLEAN",
		"ADD_SPAN_LOCATION_MISMATCHING_CRC",
		"ADD_SPAN_LOCATION_EXISTS",
		"SWAP_BLOCKS_MISMATCHING_LOCATION",
		"LOCATION_EXISTS",
		"LOCATION_NOT_FOUND",
		"AUTO_DECOMMISSION_RATE_LIMITED",
		"REGISTRY_OVERLOADED",
		"SHARDS_NOT_READY",
		"CDC_NOT_READY",
	}

	kernelShardReqResps := []ReqResp{
		{
			0x01,
			reflect.TypeOf(msgs.LookupReq{}),
			reflect.TypeOf(msgs.LookupResp{}),
		},
		{
			0x02,
			reflect.TypeOf(msgs.StatFileReq{}),
			reflect.TypeOf(msgs.StatFileResp{}),
		},
		{
			0x04,
			reflect.TypeOf(msgs.StatDirectoryReq{}),
			reflect.TypeOf(msgs.StatDirectoryResp{}),
		},
		{
			0x05,
			reflect.TypeOf(msgs.ReadDirReq{}),
			reflect.TypeOf(msgs.ReadDirResp{}),
		},
		{
			0x06,
			reflect.TypeOf(msgs.ConstructFileReq{}),
			reflect.TypeOf(msgs.ConstructFileResp{}),
		},
		{
			0x07,
			reflect.TypeOf(msgs.AddSpanInitiateReq{}),
			reflect.TypeOf(msgs.AddSpanInitiateResp{}),
		},
		{
			0x08,
			reflect.TypeOf(msgs.AddSpanCertifyReq{}),
			reflect.TypeOf(msgs.AddSpanCertifyResp{}),
		},
		{
			0x09,
			reflect.TypeOf(msgs.LinkFileReq{}),
			reflect.TypeOf(msgs.LinkFileResp{}),
		},
		{
			0x0A,
			reflect.TypeOf(msgs.SoftUnlinkFileReq{}),
			reflect.TypeOf(msgs.SoftUnlinkFileResp{}),
		},
		{
			0x0B,
			reflect.TypeOf(msgs.LocalFileSpansReq{}),
			reflect.TypeOf(msgs.LocalFileSpansResp{}),
		},
		{
			0x0C,
			reflect.TypeOf(msgs.SameDirectoryRenameReq{}),
			reflect.TypeOf(msgs.SameDirectoryRenameResp{}),
		},
		{
			0x10,
			reflect.TypeOf(msgs.AddInlineSpanReq{}),
			reflect.TypeOf(msgs.AddInlineSpanResp{}),
		},
		{
			0x11,
			reflect.TypeOf(msgs.SetTimeReq{}),
			reflect.TypeOf(msgs.SetTimeResp{}),
		},
		// this was marked as a "private" operation, but we now use it in
		// the client (to check deleted edges)
		{
			0x73,
			reflect.TypeOf(msgs.FullReadDirReq{}),
			reflect.TypeOf(msgs.FullReadDirResp{}),
		},
		// this is also "private" but we use it to stash away broken dirty
		// spans
		{
			0x7B,
			reflect.TypeOf(msgs.MoveSpanReq{}),
			reflect.TypeOf(msgs.MoveSpanResp{}),
		},
		// again, this is private, but we plan to use it to restart writing empty
		// files.
		{
			0x74,
			reflect.TypeOf(msgs.RemoveNonOwnedEdgeReq{}),
			reflect.TypeOf(msgs.RemoveNonOwnedEdgeResp{}),
		},
		{
			0x75,
			reflect.TypeOf(msgs.SameShardHardFileUnlinkReq{}),
			reflect.TypeOf(msgs.SameShardHardFileUnlinkResp{}),
		},
	}

	shardReqResps := append(kernelShardReqResps, []ReqResp{
		{
			0x03,
			reflect.TypeOf(msgs.StatTransientFileReq{}),
			reflect.TypeOf(msgs.StatTransientFileResp{}),
		},
		{
			0x12,
			reflect.TypeOf(msgs.ShardSnapshotReq{}),
			reflect.TypeOf(msgs.ShardSnapshotResp{}),
		},
		{
			0x14,
			reflect.TypeOf(msgs.FileSpansReq{}),
			reflect.TypeOf(msgs.FileSpansResp{}),
		},
		{
			0x15,
			reflect.TypeOf(msgs.AddSpanLocationReq{}),
			reflect.TypeOf(msgs.AddSpanLocationResp{}),
		},
		{
			0x16,
			reflect.TypeOf(msgs.ScrapTransientFileReq{}),
			reflect.TypeOf(msgs.ScrapTransientFileResp{}),
		},
		{
			0x0D,
			reflect.TypeOf(msgs.SetDirectoryInfoReq{}),
			reflect.TypeOf(msgs.SetDirectoryInfoResp{}),
		},
		{
			0x0E,
			reflect.TypeOf(msgs.GetLinkEntriesReq{}),
			reflect.TypeOf(msgs.GetLinkEntriesResp{}),
		},
		{
			0x0F,
			reflect.TypeOf(msgs.WaitStateAppliedReq{}),
			reflect.TypeOf(msgs.WaitStateAppliedResp{}),
		},
		// PRIVATE OPERATIONS -- These are safe operations, but we don't want the FS client itself
		// to perform them. TODO make privileged?
		{
			0x70,
			reflect.TypeOf(msgs.VisitDirectoriesReq{}),
			reflect.TypeOf(msgs.VisitDirectoriesResp{}),
		},
		{
			0x71,
			reflect.TypeOf(msgs.VisitFilesReq{}),
			reflect.TypeOf(msgs.VisitFilesResp{}),
		},
		{
			0x72,
			reflect.TypeOf(msgs.VisitTransientFilesReq{}),
			reflect.TypeOf(msgs.VisitTransientFilesResp{}),
		},
		{
			0x76,
			reflect.TypeOf(msgs.RemoveSpanInitiateReq{}),
			reflect.TypeOf(msgs.RemoveSpanInitiateResp{}),
		},
		{
			0x77,
			reflect.TypeOf(msgs.RemoveSpanCertifyReq{}),
			reflect.TypeOf(msgs.RemoveSpanCertifyResp{}),
		},
		{
			0x78,
			reflect.TypeOf(msgs.SwapBlocksReq{}),
			reflect.TypeOf(msgs.SwapBlocksResp{}),
		},
		{
			0x79,
			reflect.TypeOf(msgs.BlockServiceFilesReq{}),
			reflect.TypeOf(msgs.BlockServiceFilesResp{}),
		},
		{
			0x7A,
			reflect.TypeOf(msgs.RemoveInodeReq{}),
			reflect.TypeOf(msgs.RemoveInodeResp{}),
		},
		{
			0x7C,
			reflect.TypeOf(msgs.AddSpanInitiateWithReferenceReq{}),
			reflect.TypeOf(msgs.AddSpanInitiateWithReferenceResp{}),
		},
		{
			0x7D,
			reflect.TypeOf(msgs.RemoveZeroBlockServiceFilesReq{}),
			reflect.TypeOf(msgs.RemoveZeroBlockServiceFilesResp{}),
		},
		{
			0x7E,
			reflect.TypeOf(msgs.SwapSpansReq{}),
			reflect.TypeOf(msgs.SwapSpansResp{}),
		},
		{
			0x7F,
			reflect.TypeOf(msgs.SameDirectoryRenameSnapshotReq{}),
			reflect.TypeOf(msgs.SameDirectoryRenameSnapshotResp{}),
		},
		// Needs to be after AddSpanInitiateWithReferenceReq
		{
			0x13,
			reflect.TypeOf(msgs.AddSpanAtLocationInitiateReq{}),
			reflect.TypeOf(msgs.AddSpanAtLocationInitiateResp{}),
		},
		// UNSAFE OPERATIONS -- these can break invariants.
		{
			0x80,
			reflect.TypeOf(msgs.CreateDirectoryInodeReq{}),
			reflect.TypeOf(msgs.CreateDirectoryInodeResp{}),
		},
		{
			0x81,
			reflect.TypeOf(msgs.SetDirectoryOwnerReq{}),
			reflect.TypeOf(msgs.SetDirectoryOwnerResp{}),
		},
		{
			0x89,
			reflect.TypeOf(msgs.RemoveDirectoryOwnerReq{}),
			reflect.TypeOf(msgs.RemoveDirectoryOwnerResp{}),
		},
		{
			0x82,
			reflect.TypeOf(msgs.CreateLockedCurrentEdgeReq{}),
			reflect.TypeOf(msgs.CreateLockedCurrentEdgeResp{}),
		},
		{
			0x83,
			reflect.TypeOf(msgs.LockCurrentEdgeReq{}),
			reflect.TypeOf(msgs.LockCurrentEdgeResp{}),
		},
		{
			0x84,
			reflect.TypeOf(msgs.UnlockCurrentEdgeReq{}),
			reflect.TypeOf(msgs.UnlockCurrentEdgeResp{}),
		},
		{
			0x86,
			reflect.TypeOf(msgs.RemoveOwnedSnapshotFileEdgeReq{}),
			reflect.TypeOf(msgs.RemoveOwnedSnapshotFileEdgeResp{}),
		},
		{
			0x87,
			reflect.TypeOf(msgs.MakeFileTransientReq{}),
			reflect.TypeOf(msgs.MakeFileTransientResp{}),
		},
	}...)

	kernelCdcReqResps := []ReqResp{
		{
			0x01,
			reflect.TypeOf(msgs.MakeDirectoryReq{}),
			reflect.TypeOf(msgs.MakeDirectoryResp{}),
		},
		{
			0x02,
			reflect.TypeOf(msgs.RenameFileReq{}),
			reflect.TypeOf(msgs.RenameFileResp{}),
		},
		{
			0x03,
			reflect.TypeOf(msgs.SoftUnlinkDirectoryReq{}),
			reflect.TypeOf(msgs.SoftUnlinkDirectoryResp{}),
		},
		{
			0x04,
			reflect.TypeOf(msgs.RenameDirectoryReq{}),
			reflect.TypeOf(msgs.RenameDirectoryResp{}),
		},
		{
			0x06,
			reflect.TypeOf(msgs.CrossShardHardUnlinkFileReq{}),
			reflect.TypeOf(msgs.CrossShardHardUnlinkFileResp{}),
		},
	}

	cdcReqResps := append(kernelCdcReqResps, []ReqResp{
		{
			0x05,
			reflect.TypeOf(msgs.HardUnlinkDirectoryReq{}),
			reflect.TypeOf(msgs.HardUnlinkDirectoryResp{}),
		},
		{
			0x07,
			reflect.TypeOf(msgs.CdcSnapshotReq{}),
			reflect.TypeOf(msgs.CdcSnapshotResp{}),
		},
	}...)

	kernelRegistryReqResps := []ReqResp{
		{
			0x03,
			reflect.TypeOf(msgs.LocalShardsReq{}),
			reflect.TypeOf(msgs.LocalShardsResp{}),
		},
		{
			0x07,
			reflect.TypeOf(msgs.LocalCdcReq{}),
			reflect.TypeOf(msgs.LocalCdcResp{}),
		},
		{
			0x08,
			reflect.TypeOf(msgs.InfoReq{}),
			reflect.TypeOf(msgs.InfoResp{}),
		},
		{
			0x0F,
			reflect.TypeOf(msgs.RegistryReq{}),
			reflect.TypeOf(msgs.RegistryResp{}),
		},
		{
			0x22,
			reflect.TypeOf(msgs.LocalChangedBlockServicesReq{}),
			reflect.TypeOf(msgs.LocalChangedBlockServicesResp{}),
		},
	}

	registryReqResps := append(kernelRegistryReqResps, []ReqResp{
		{
			0x01,
			reflect.TypeOf(msgs.CreateLocationReq{}),
			reflect.TypeOf(msgs.CreateLocationResp{}),
		},
		{
			0x02,
			reflect.TypeOf(msgs.RenameLocationReq{}),
			reflect.TypeOf(msgs.RenameLocationResp{}),
		},
		{
			0x04,
			reflect.TypeOf(msgs.RegisterShardReq{}),
			reflect.TypeOf(msgs.RegisterShardResp{}),
		},
		{
			0x05,
			reflect.TypeOf(msgs.LocationsReq{}),
			reflect.TypeOf(msgs.LocationsResp{}),
		},
		{
			0x06,
			reflect.TypeOf(msgs.RegisterCdcReq{}),
			reflect.TypeOf(msgs.RegisterCdcResp{}),
		},
		{
			0x09,
			reflect.TypeOf(msgs.SetBlockServiceFlagsReq{}),
			reflect.TypeOf(msgs.SetBlockServiceFlagsResp{}),
		},
		{
			0x0A,
			reflect.TypeOf(msgs.RegisterBlockServicesReq{}),
			reflect.TypeOf(msgs.RegisterBlockServicesResp{}),
		},
		{
			0x0B,
			reflect.TypeOf(msgs.ChangedBlockServicesAtLocationReq{}),
			reflect.TypeOf(msgs.ChangedBlockServicesAtLocationResp{}),
		},
		{
			0x0C,
			reflect.TypeOf(msgs.ShardsAtLocationReq{}),
			reflect.TypeOf(msgs.ShardsAtLocationResp{}),
		},
		{
			0x0D,
			reflect.TypeOf(msgs.CdcAtLocationReq{}),
			reflect.TypeOf(msgs.CdcAtLocationResp{}),
		},
		{
			0x0E,
			reflect.TypeOf(msgs.RegisterRegistryReq{}),
			reflect.TypeOf(msgs.RegisterRegistryResp{}),
		},
		{
			0x10,
			reflect.TypeOf(msgs.AllRegistryReplicasDEPRECATEDReq{}),
			reflect.TypeOf(msgs.AllRegistryReplicasDEPRECATEDResp{}),
		},
		{
			0x11,
			reflect.TypeOf(msgs.ShardBlockServicesDEPRECATEDReq{}),
			reflect.TypeOf(msgs.ShardBlockServicesDEPRECATEDResp{}),
		},
		{
			0x13,
			reflect.TypeOf(msgs.CdcReplicasDEPRECATEDReq{}),
			reflect.TypeOf(msgs.CdcReplicasDEPRECATEDResp{}),
		},
		{
			0x14,
			reflect.TypeOf(msgs.AllShardsDEPRECATEDReq{}),
			reflect.TypeOf(msgs.AllShardsDEPRECATEDResp{}),
		},
		{
			0x15,
			reflect.TypeOf(msgs.DecommissionBlockServiceReq{}),
			reflect.TypeOf(msgs.DecommissionBlockServiceResp{}),
		},
		{
			0x16,
			reflect.TypeOf(msgs.MoveShardLeaderReq{}),
			reflect.TypeOf(msgs.MoveShardLeaderResp{}),
		},
		{
			0x17,
			reflect.TypeOf(msgs.ClearShardInfoReq{}),
			reflect.TypeOf(msgs.ClearShardInfoResp{}),
		},
		{
			0x18,
			reflect.TypeOf(msgs.ShardBlockServicesReq{}),
			reflect.TypeOf(msgs.ShardBlockServicesResp{}),
		},
		{
			0x19,
			reflect.TypeOf(msgs.AllCdcDEPRECATEDReq{}),
			reflect.TypeOf(msgs.AllCdcDEPRECATEDResp{}),
		},
		{
			0x20,
			reflect.TypeOf(msgs.EraseDecommissionedBlockReq{}),
			reflect.TypeOf(msgs.EraseDecommissionedBlockResp{}),
		},
		{
			0x21,
			reflect.TypeOf(msgs.AllBlockServicesDeprecatedReq{}),
			reflect.TypeOf(msgs.AllBlockServicesDeprecatedResp{}),
		},
		{
			0x28,
			reflect.TypeOf(msgs.AllBlockServicesReq{}),
			reflect.TypeOf(msgs.AllBlockServicesResp{}),
		},
		{
			0x23,
			reflect.TypeOf(msgs.MoveCdcLeaderReq{}),
			reflect.TypeOf(msgs.MoveCdcLeaderResp{}),
		},
		{
			0x24,
			reflect.TypeOf(msgs.ClearCdcInfoReq{}),
			reflect.TypeOf(msgs.ClearCdcInfoResp{}),
		},
		{
			0x25,
			reflect.TypeOf(msgs.UpdateBlockServicePathReq{}),
			reflect.TypeOf(msgs.UpdateBlockServicePathResp{}),
		},
		{
			0x26,
			reflect.TypeOf(msgs.SetBlockServiceHasFilesReq{}),
			reflect.TypeOf(msgs.SetBlockServiceHasFilesResp{}),
		},
		{
			0x27,
			reflect.TypeOf(msgs.BlockServicesNeedingMigrationDeprecatedReq{}),
			reflect.TypeOf(msgs.BlockServicesNeedingMigrationDeprecatedResp{}),
		},
		{
			0x29,
			reflect.TypeOf(msgs.AllRegistryReplicasReq{}),
			reflect.TypeOf(msgs.AllRegistryReplicasResp{}),
		},
		{
			0x2A,
			reflect.TypeOf(msgs.AllShardsReq{}),
			reflect.TypeOf(msgs.AllShardsResp{}),
		},
		{
			0x2B,
			reflect.TypeOf(msgs.AllCdcReq{}),
			reflect.TypeOf(msgs.AllCdcResp{}),
		},
		{
			0x2C,
			reflect.TypeOf(msgs.ChangedBlockServicesDeprecatedReq{}),
			reflect.TypeOf(msgs.ChangedBlockServicesDeprecatedResp{}),
		},
		{
			0x2D,
			reflect.TypeOf(msgs.BlockServiceAvailableSpaceDeprecatedReq{}),
			reflect.TypeOf(msgs.BlockServiceAvailableSpaceDeprecatedResp{}),
		},
		{
			0x2E,
			reflect.TypeOf(msgs.ChangedBlockServicesReq{}),
			reflect.TypeOf(msgs.ChangedBlockServicesResp{}),
		},
		{
			0x2F,
			reflect.TypeOf(msgs.BlockServicesNeedingMigrationReq{}),
			reflect.TypeOf(msgs.BlockServicesNeedingMigrationResp{}),
		},
		{
			0x30,
			reflect.TypeOf(msgs.BlockServiceAvailableSpaceReq{}),
			reflect.TypeOf(msgs.BlockServiceAvailableSpaceResp{}),
		},
		{
			0x31,
			reflect.TypeOf(msgs.LocalShardReplicasReq{}),
			reflect.TypeOf(msgs.LocalShardReplicasResp{}),
		},
	}...)

	kernelBlocksReqResps := []ReqResp{
		{
			0x02,
			reflect.TypeOf(msgs.FetchBlockReq{}),
			reflect.TypeOf(msgs.FetchBlockResp{}),
		},
		{
			0x03,
			reflect.TypeOf(msgs.WriteBlockReq{}),
			reflect.TypeOf(msgs.WriteBlockResp{}),
		},
		{
			0x04,
			reflect.TypeOf(msgs.FetchBlockWithCrcReq{}),
			reflect.TypeOf(msgs.FetchBlockWithCrcResp{}),
		},
	}

	blocksReqResps := append(kernelBlocksReqResps, []ReqResp{
		{
			0x01,
			reflect.TypeOf(msgs.EraseBlockReq{}),
			reflect.TypeOf(msgs.EraseBlockResp{}),
		},
		{
			0x05,
			reflect.TypeOf(msgs.TestWriteReq{}),
			reflect.TypeOf(msgs.TestWriteResp{}),
		},
		{
			0x06,
			reflect.TypeOf(msgs.CheckBlockReq{}),
			reflect.TypeOf(msgs.CheckBlockResp{}),
		},
	}...)

	logReqResps := []ReqResp{
		{
			0x01,
			reflect.TypeOf(msgs.LogWriteReq{}),
			reflect.TypeOf(msgs.LogWriteResp{}),
		},
		{
			0x02,
			reflect.TypeOf(msgs.ReleaseReq{}),
			reflect.TypeOf(msgs.ReleaseResp{}),
		},
		{
			0x03,
			reflect.TypeOf(msgs.LogReadReq{}),
			reflect.TypeOf(msgs.LogReadResp{}),
		},
		{
			0x04,
			reflect.TypeOf(msgs.NewLeaderReq{}),
			reflect.TypeOf(msgs.NewLeaderResp{}),
		},
		{
			0x05,
			reflect.TypeOf(msgs.NewLeaderConfirmReq{}),
			reflect.TypeOf(msgs.NewLeaderConfirmResp{}),
		},
		{
			0x06,
			reflect.TypeOf(msgs.LogRecoveryReadReq{}),
			reflect.TypeOf(msgs.LogRecoveryReadResp{}),
		},
		{
			0x07,
			reflect.TypeOf(msgs.LogRecoveryWriteReq{}),
			reflect.TypeOf(msgs.LogRecoveryWriteResp{}),
		},
	}

	kernelExtras := []reflect.Type{
		reflect.TypeOf(msgs.DirectoryInfoEntry{}),
		reflect.TypeOf(msgs.DirectoryInfo{}),
		reflect.TypeOf(msgs.CurrentEdge{}),
		reflect.TypeOf(msgs.AddSpanInitiateBlockInfo{}),
		reflect.TypeOf(msgs.RemoveSpanInitiateBlockInfo{}),
		reflect.TypeOf(msgs.BlockProof{}),
		reflect.TypeOf(msgs.BlockService{}),
		reflect.TypeOf(msgs.ShardInfo{}),
		reflect.TypeOf(msgs.BlockPolicyEntry{}),
		reflect.TypeOf(msgs.SpanPolicyEntry{}),
		reflect.TypeOf(msgs.StripePolicy{}),
		reflect.TypeOf(msgs.FetchedBlock{}),
		reflect.TypeOf(msgs.FetchedSpanHeader{}),
		reflect.TypeOf(msgs.FetchedInlineSpan{}),
		reflect.TypeOf(msgs.FetchedBlocksSpan{}),
		reflect.TypeOf(msgs.FetchedBlockServices{}),
		reflect.TypeOf(msgs.FetchedLocations{}),
		reflect.TypeOf(msgs.FetchedSpanHeaderFull{}),
		reflect.TypeOf(msgs.BlacklistEntry{}),
		reflect.TypeOf(msgs.Edge{}),
		reflect.TypeOf(msgs.FullReadDirCursor{}),
	}

	extras := append(
		[]reflect.Type{
			reflect.TypeOf(msgs.FailureDomain{}),
		},
		append(kernelExtras, []reflect.Type{
			reflect.TypeOf(msgs.LinkEntry{}),
			reflect.TypeOf(msgs.FullRegistryInfo{}),
			reflect.TypeOf(msgs.TransientFile{}),
			reflect.TypeOf(msgs.EntryNewBlockInfo{}),
			reflect.TypeOf(msgs.BlockServiceDeprecatedInfo{}),
			reflect.TypeOf(msgs.BlockServiceInfoShort{}),
			reflect.TypeOf(msgs.SpanPolicy{}),
			reflect.TypeOf(msgs.BlockPolicy{}),
			reflect.TypeOf(msgs.SnapshotPolicy{}),
			reflect.TypeOf(msgs.FullShardInfo{}),
			reflect.TypeOf(msgs.RegisterBlockServiceInfo{}),
			reflect.TypeOf(msgs.FullBlockServiceInfo{}),
			reflect.TypeOf(msgs.BlockServiceSpace{}),
			reflect.TypeOf(msgs.CdcInfo{}),
			reflect.TypeOf(msgs.LocationInfo{}),
		}...)...)

	goExtras := append(extras, []reflect.Type{
		reflect.TypeOf(msgs.IpPort{}),
		reflect.TypeOf(msgs.AddrsInfo{}),
	}...)

	kernelExtras = append([]reflect.Type{
		reflect.TypeOf(msgs.IpPort{}),
		reflect.TypeOf(msgs.AddrsInfo{})},
		kernelExtras...)

	_ = logReqResps
	return Lists{errors, shardReqResps, cdcReqResps, registryReqResps, blocksReqResps, goExtras}
}

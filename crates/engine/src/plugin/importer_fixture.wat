;; Original test component for the importer world, written by hand in the component text
;; format. Each test supplies the core `$import` body and an optional extra import.
(component
  (type $ty-types (instance
    (type (;0;) string)
    (export (;1;) "json" (type (eq 0)))
    (type (;2;) (variant (case "failed" string) (case "denied" string) (case "limit" string) (case "invalid" string)))
    (export (;3;) "plugin-error" (type (eq 2)))
  ))
  (import "solidcraft:plugin/types@0.1.0" (instance $types (type $ty-types)))
  (alias export $types "plugin-error" (type $plugin-error))
  ;; EXTRA-IMPORT
  (core module $main
    (memory (;0;) 16)
    (global $heap (mut i32) (i32.const 4096))
    (export "memory" (memory 0))
    (export "cabi_realloc" (func $realloc))
    (export "solidcraft:plugin/importer@0.1.0#import" (func $import))
    (func $realloc (param i32 i32 i32 i32) (result i32)
      (local $at i32)
      global.get $heap
      local.tee $at
      local.get 3
      i32.add
      i32.const 7
      i32.add
      i32.const -8
      i32.and
      global.set $heap
      local.get $at
    )
    ;; file-name (ptr, len), bytes (ptr, len) -> pointer to the result.
    ;; Result layout: 0 ok/err; 4 payload case; 8, 12 payload list; 16, 20 notes.
    (func $import (param $name i32) (param $name_len i32) (param $bytes i32) (param $bytes_len i32) (result i32)
      ;; BODY
    )
  )
  (core instance $main (instantiate $main))
  (alias core export $main "memory" (core memory $memory))
  (alias core export $main "cabi_realloc" (core func $realloc))
  (type $bytes (list u8))
  (type $payload (variant (case "design" $bytes) (case "step" $bytes)))
  (type $imported (record (field "payload" $payload) (field "notes" string)))
  (type $result (result $imported (error $plugin-error)))
  (type $import-type (func (param "file-name" string) (param "bytes" $bytes) (result $result)))
  (alias core export $main "solidcraft:plugin/importer@0.1.0#import" (core func $core-import))
  (func $import (type $import-type) (canon lift (core func $core-import) (memory $memory) (realloc $realloc) string-encoding=utf8))
  (component $shim
    (type (;0;) (variant (case "failed" string) (case "denied" string) (case "limit" string) (case "invalid" string)))
    (import "import-type-plugin-error" (type (;1;) (eq 0)))
    (type (;2;) (list u8))
    (type (;3;) (variant (case "design" 2) (case "step" 2)))
    (import "import-type-payload" (type (;4;) (eq 3)))
    (type (;5;) (record (field "payload" 4) (field "notes" string)))
    (import "import-type-imported" (type (;6;) (eq 5)))
    (type (;7;) (result 6 (error 1)))
    (type (;8;) (func (param "file-name" string) (param "bytes" 2) (result 7)))
    (import "import-func-import" (func (;0;) (type 8)))
    (export (;9;) "plugin-error" (type 1))
    (export (;10;) "payload" (type 4))
    (type (;11;) (record (field "payload" 10) (field "notes" string)))
    (export (;12;) "imported" (type 11))
    (type (;13;) (result 12 (error 9)))
    (type (;14;) (func (param "file-name" string) (param "bytes" 2) (result 13)))
    (export (;1;) "import" (func 0) (func (type 14)))
  )
  (instance $exports (instantiate $shim
    (with "import-func-import" (func $import))
    (with "import-type-plugin-error" (type $plugin-error))
    (with "import-type-payload" (type $payload))
    (with "import-type-imported" (type $imported))
  ))
  (export "solidcraft:plugin/importer@0.1.0" (instance $exports))
)

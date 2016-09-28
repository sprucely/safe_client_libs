initSidebarItems({"constant":[["UNVERSIONED_STRUCT_DATA_TYPE_TAG","Unversioned StructuredData"],["VERSIONED_STRUCT_DATA_TYPE_TAG","Versioned StructuredData"]],"mod":[["core","Core module"],["dns","Dns module;"],["ffi","Ffi module; This module provides FFI-bindings to the Client Modules (`core`, `nfs`, `dns`) In the current implementation the allocations made by this crate are managed within the crate itself and is guaranteed that management of such allocations will not be pushed beyond the FFI boundary. This has a 2-fold outcome: firstly, the passing of data is done by filling of the allocations passed by the caller and is caller's responsibility to manage those. For this every function that fills an allocated memory also has a companion function to return the size of data which the caller can call to find out how much space needs to be allocated in the first place. Second and consequently, the caller does not have to bother calling functions within this crate which only serve to free resources allocated by the crate itself. This otherwise would be error prone and cumbersome. Instead the caller can use whatever idiom in his language to manage memory much more naturally and conveniently (eg., RAII idioms etc)"],["nfs","Nfs module;"]]});
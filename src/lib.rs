#![allow(unused_imports)]
#![allow(non_camel_case_types)]
#![allow(unused_variables)]
#![allow(unused_must_use)]
#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]

#[macro_use] extern crate bitflags;
#[macro_use] extern crate custom_derive;
#[cfg(test)] #[macro_use] extern crate grabbag_macros;
#[macro_use] extern crate log;
#[macro_use] extern crate newtype_derive;

#[cfg(feature = "dynamodb_tests")]
extern crate hyper;
#[cfg(feature = "dynamodb_tests")]
extern crate rusoto;

//#[cfg(test)]
//extern crate test;

extern crate rustc_serialize;
extern crate mio;
extern crate nix;
extern crate net2;
extern crate time;
extern crate rand;
extern crate uuid;
extern crate libc;

//FIXME only needed until repeated multiput returns is fixed
extern crate linked_hash_map;

#[macro_use]
mod general_tests;

pub mod storeables;
pub mod packets;
pub mod prelude;
pub mod local_store;
pub mod udp_store;
pub mod tcp_store;
pub mod multitcp_store;
pub mod servers;
pub mod color_api;

#[cfg(feature = "dynamodb_tests")]
pub mod dynamo_store;

pub mod c_binidings {

    use prelude::*;
    use local_store::LocalHorizon;
    use tcp_store::TcpStore;
    //TODO use multitcp_store::TcpStore;

    use std::collections::HashMap;
    use std::{mem, ptr, slice};

    use std::ffi::CStr;
    use std::os::raw::c_char;

    use color_api::*;

    pub type DAG = DAGHandle<[u8], TcpStore<[u8]>, LocalHorizon>;
    pub type ColorID = u32;

    #[repr(C)]
    pub struct colors {
        numcolors: usize,
        mycolors: *const ColorID,
    }

    #[no_mangle]
    pub extern "C" fn new_dag_handle(num_ips: usize, server_ips: *const *const i8,
        color: *const colors) -> Box<DAG> {
        assert_eq!(mem::size_of::<Box<DAG>>(), mem::size_of::<*mut u8>());
        assert_eq!(num_ips, 1, "Multiple servers are not yet supported via the C API");
        assert!(server_ips != ptr::null());
        assert!(unsafe {*server_ips != ptr::null()});
        assert!(color != ptr::null());
        assert!(colors_valid(color));

        let server_addr_str = unsafe { CStr::from_ptr(*server_ips).to_str().expect("invalid IP string") };
        let ip_addr = server_addr_str.parse().expect("invalid IP addr");
        let colors = unsafe {slice::from_raw_parts((*color).mycolors, (*color).numcolors)};
        Box::new(DAGHandle::new(TcpStore::new(ip_addr), LocalHorizon::new(), colors))
    }

    //NOTE currently can only use 31bits of return value
    #[no_mangle]
    pub extern "C" fn append(dag: *mut DAG, data: *const u8, data_size: usize,
        inhabits: *const colors, depends_on: *const colors) -> u32 {
        assert!(data_size == 0 || data != ptr::null());
        assert!(inhabits != ptr::null());
        assert!(colors_valid(inhabits));
        assert!(data_size <= 8000);

        let (dag, data, inhabits) = unsafe {
            (dag.as_mut().expect("need to provide a valid DAGHandle"),
                slice::from_raw_parts(data, data_size),
                slice::from_raw_parts((*inhabits).mycolors, (*inhabits).numcolors))
        };
        let depends_on = unsafe {
            if depends_on != ptr::null() {
                assert!(colors_valid(depends_on));
                slice::from_raw_parts((*depends_on).mycolors, (*depends_on).numcolors)
            }
            else {
                &[]
            }
        };
        dag.append(data, inhabits, depends_on);
        0
    }

    fn colors_valid(c: *const colors) -> bool {
        unsafe { c != ptr::null() &&
            ((*c).numcolors == 0 || (*c).mycolors != ptr::null()) }
    }

    //NOTE we need either a way to specify data size, or to pass out a pointer
    // this version simple assumes that no data+metadat passed in or out will be
    // greater than DELOS_MAX_DATA_SIZE
    #[no_mangle]
    pub extern "C" fn get_next(dag: *mut DAG, data_out: *mut u8, data_read: *mut usize,
        inhabits_out: *mut colors) -> u32 {
        assert!(data_out != ptr::null_mut());
        assert!(data_read != ptr::null_mut());
        assert!(inhabits_out != ptr::null_mut());

        let dag = unsafe {dag.as_mut().expect("need to provide a valid DAGHandle")};
        let data_out = unsafe { slice::from_raw_parts_mut(data_out, 8000)};
        let data_read = unsafe {data_read.as_mut().expect("must provide valid data_out")};
        let inhabited_colors = dag.get_next(data_out, data_read);
        unsafe {
            let numcolors = inhabited_colors.len();
            let mut mycolors = ptr::null_mut();
            if numcolors != 0 {
                mycolors = ::libc::malloc(mem::size_of::<ColorID>() * numcolors) as *mut _;
                ptr::copy_nonoverlapping(&inhabited_colors[0], mycolors, numcolors);
            }
            ptr::write(inhabits_out, colors{ numcolors: numcolors, mycolors: mycolors});
        };
        0
    }

    #[no_mangle]
    pub extern "C" fn snapshot(dag: *mut DAG) {
        let dag = unsafe {dag.as_mut().expect("need to provide a valid DAGHandle")};
        dag.take_snapshot();
    }

    #[no_mangle]
    pub unsafe extern "C" fn close_dag_handle(dag: *mut DAG) {
        assert!(dag != ptr::null_mut());
        Box::from_raw(dag);
    }

    ////////////////////////////////////
    //         Server bindings        //
    ////////////////////////////////////

    #[no_mangle]
    pub extern "C" fn start_fuzzy_log_server(server_ip: *const i8) -> ! {
        let server_addr_str = unsafe { CStr::from_ptr(server_ip).to_str().expect("invalid IP string")
        };
        let ip_addr = server_addr_str.parse().expect("invalid IP addr");
        let mut event_loop = ::mio::EventLoop::new()
            .expect("unable to start server loop");
        let mut server = ::servers::tcp::Server::new(&ip_addr, 0, 1, &mut event_loop)
            .expect("unable to start server");
        let res = event_loop.run(&mut server);
        panic!("server stopped with: {:?}", res)
    }

    #[no_mangle]
    pub extern "C" fn start_fuzzy_log_server_thread(server_ip: *const i8) {
        use std::sync::atomic::{AtomicBool, Ordering};

        let server_started = AtomicBool::new(false);
        let started = unsafe {
            //This should be safe since the while loop at the of the function
            //prevents it from exiting until the server is started and
            //server_started is no longer used
            extend_lifetime(&server_started)
        };
        let server_addr_str = unsafe { CStr::from_ptr(server_ip).to_str().expect("invalid IP string")
        };
        let handle = ::std::thread::spawn(move || {
            let ip_addr = server_addr_str.parse().expect("invalid IP addr");
            let mut event_loop = ::mio::EventLoop::new()
                .expect("unable to start server loop");
            let mut server = ::servers::tcp::Server::new(&ip_addr, 0, 1, &mut event_loop)
                .expect("unable to start server");
            started.store(true, Ordering::SeqCst);
            mem::drop(started);
            let res = event_loop.run(&mut server);
            panic!("server stopped with: {:?}", res)
        });
        while !server_started.load(Ordering::SeqCst) {}
        ::std::mem::forget(handle);

        unsafe fn extend_lifetime<'a, 'b, T>(r: &'a T) -> &'b T {
            ::std::mem::transmute(r)
        }
    }

    ////////////////////////////////////
    //    Old fuzzy log C bindings    //
    ////////////////////////////////////

    pub type Log = FuzzyLog<[u8], TcpStore<[u8]>, LocalHorizon>;

    #[no_mangle]
    pub extern "C" fn fuzzy_log_new(server_addr: *const c_char, relevent_chains: *const u32,
        num_relevent_chains: u16, callback: extern fn(*const u8, u16) -> u8) -> Box<Log> {
        let mut callbacks = HashMap::new();
        let relevent_chains = unsafe { slice::from_raw_parts(relevent_chains, num_relevent_chains as usize) };
        for &chain in relevent_chains {
            let callback: Box<Fn(&Uuid, &OrderIndex, &[u8]) -> bool> = Box::new(move |_, _, val| { callback(&val[0], val.len() as u16) != 0 });
            callbacks.insert(chain.into(), callback);
        }
        let server_addr_str = unsafe { CStr::from_ptr(server_addr).to_str().expect("invalid IP string") };
        let ip_addr = server_addr_str.parse().expect("invalid IP addr");
        let log = FuzzyLog::new(TcpStore::new(ip_addr), LocalHorizon::new(), callbacks);
        Box::new(log)
    }

    #[no_mangle]
    pub extern "C" fn fuzzy_log_append(log: &mut Log,
        chain: u32, val: *const u8, len: u16, deps: *const OrderIndex, num_deps: u16) -> OrderIndex {
        unsafe {
            let val = slice::from_raw_parts(val, len as usize);
            let deps = slice::from_raw_parts(deps, num_deps as usize);
            log.append(chain.into(), val, deps)
        }
    }

    #[no_mangle]
    pub extern "C" fn fuzzy_log_multiappend(log: &mut Log,
        chains: *mut OrderIndex, num_chains: u16,
        val: *const u8, len: u16, deps: *const OrderIndex, num_deps: u16) {
        assert!(num_chains > 1);
        unsafe {
            let val = slice::from_raw_parts(val, len as usize);
            let deps = slice::from_raw_parts(deps, num_deps as usize);
            let chains = slice::from_raw_parts_mut(chains, num_chains as usize);
            log.multiappend2(chains, val, deps);
        }
    }

    #[no_mangle]
    pub extern "C" fn fuzzy_log_play_forward(log: &mut Log, chain: u32) -> OrderIndex {
        if let Some(oi) = log.play_foward(order::from(chain)) {
            oi
        }
        else {
            (0.into(), 0.into())
        }
    }

}

#![allow(non_camel_case_types)]

pub mod org {
    pub mod hiero {
        pub mod block {
            pub mod api {
                tonic::include_proto!("org.hiero.block.api");
            }
        }
    }
}

pub mod com {
    pub mod hedera {
        pub mod hapi {
            pub mod block {
                pub mod stream {
                    tonic::include_proto!("com.hedera.hapi.block.stream");
                    pub mod input {
                        tonic::include_proto!("com.hedera.hapi.block.stream.input");
                    }
                    pub mod output {
                        tonic::include_proto!("com.hedera.hapi.block.stream.output");
                    }
                }
            }
            pub mod node {
                pub mod addressbook {
                    tonic::include_proto!("com.hedera.hapi.node.addressbook");
                }
                pub mod state {
                    pub mod addressbook {
                        tonic::include_proto!("com.hedera.hapi.node.state.addressbook");
                    }
                    pub mod blockstream {
                        tonic::include_proto!("com.hedera.hapi.node.state.blockstream");
                    }
                    pub mod entity {
                        tonic::include_proto!("com.hedera.hapi.node.state.entity");
                    }
                    pub mod history {
                        tonic::include_proto!("com.hedera.hapi.node.state.history");
                    }
                    pub mod hints {
                        tonic::include_proto!("com.hedera.hapi.node.state.hints");
                    }
                    pub mod roster {
                        tonic::include_proto!("com.hedera.hapi.node.state.roster");
                    }
                    pub mod tss {
                        tonic::include_proto!("com.hedera.hapi.node.state.tss");
                    }
                }
            }
            pub mod platform {
                pub mod event {
                    tonic::include_proto!("com.hedera.hapi.platform.event");
                }
                pub mod state {
                    tonic::include_proto!("com.hedera.hapi.platform.state");
                }
            }
            pub mod services {
                pub mod auxiliary {
                    pub mod history {
                        tonic::include_proto!("com.hedera.hapi.services.auxiliary.history");
                    }
                    pub mod hints {
                        tonic::include_proto!("com.hedera.hapi.services.auxiliary.hints");
                    }
                    pub mod tss {
                        tonic::include_proto!("com.hedera.hapi.services.auxiliary.tss");
                    }
                }
            }
        }
        pub mod mirror {
            pub mod api {
                pub mod proto {
                    tonic::include_proto!("com.hedera.mirror.api.proto");
                }
            }
        }
    }
}

pub mod proto {
    tonic::include_proto!("proto");
}

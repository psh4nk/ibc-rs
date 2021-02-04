mod modelator;
mod step;

use ibc::ics02_client::client_def::{AnyClientState, AnyConsensusState};
use ibc::ics02_client::client_type::ClientType;
use ibc::ics02_client::error::Kind as ICS02ErrorKind;
use ibc::ics02_client::msgs::create_client::MsgCreateAnyClient;
use ibc::ics02_client::msgs::update_client::MsgUpdateAnyClient;
use ibc::ics02_client::msgs::ClientMsg;
use ibc::ics03_connection::connection::Counterparty;
use ibc::ics03_connection::error::Kind as ICS03ErrorKind;
use ibc::ics03_connection::msgs::conn_open_init::MsgConnectionOpenInit;
use ibc::ics03_connection::msgs::conn_open_try::MsgConnectionOpenTry;
use ibc::ics03_connection::msgs::ConnectionMsg;
use ibc::ics03_connection::version::Version;
use ibc::ics18_relayer::error::{Error as ICS18Error, Kind as ICS18ErrorKind};
use ibc::ics23_commitment::commitment::CommitmentPrefix;
use ibc::ics23_commitment::commitment::CommitmentProofBytes;
use ibc::ics24_host::identifier::{ChainId, ClientId, ConnectionId};
use ibc::ics26_routing::error::{Error as ICS26Error, Kind as ICS26ErrorKind};
use ibc::ics26_routing::msgs::ICS26Envelope;
use ibc::mock::client_state::{MockClientState, MockConsensusState};
use ibc::mock::context::MockContext;
use ibc::mock::header::MockHeader;
use ibc::mock::host::HostType;
use ibc::proofs::{ConsensusProof, Proofs};
use ibc::Height;
use ibc::{ics02_client::client_def::AnyHeader, ics18_relayer::context::ICS18Context};
use std::error::Error;
use std::fmt::{Debug, Display};
use std::{collections::HashMap, vec};
use step::{ActionOutcome, ActionType, Chain, Step};
use tendermint::account::Id as AccountId;

#[derive(Debug)]
struct ICS02TestExecutor {
    // mapping from chain identifier to its context
    contexts: HashMap<String, MockContext>,
}

impl ICS02TestExecutor {
    fn new() -> Self {
        Self {
            contexts: Default::default(),
        }
    }

    /// Create a `MockContext` for a given `chain_id`.
    /// Panic if a context for `chain_id` already exists.
    fn init_chain_context(&mut self, chain_id: String, initial_height: u64) {
        let max_history_size = 1;
        let ctx = MockContext::new(
            ChainId::new(chain_id.clone(), Self::epoch()),
            HostType::Mock,
            max_history_size,
            Height::new(Self::epoch(), initial_height),
        );
        assert!(self.contexts.insert(chain_id, ctx).is_none());
    }

    /// Returns a reference to the `MockContext` of a given `chain_id`.
    /// Panic if the context for `chain_id` is not found.
    fn chain_context(&self, chain_id: &String) -> &MockContext {
        self.contexts
            .get(chain_id)
            .expect("chain context should have been initialized")
    }

    /// Returns a mutable reference to the `MockContext` of a given `chain_id`.
    /// Panic if the context for `chain_id` is not found.
    fn chain_context_mut(&mut self, chain_id: &String) -> &mut MockContext {
        self.contexts
            .get_mut(chain_id)
            .expect("chain context should have been initialized")
    }

    fn extract_handler_error_kind<K>(ics18_result: Result<(), ICS18Error>) -> K
    where
        K: Clone + Debug + Display + Into<anomaly::BoxError> + 'static,
    {
        let ics18_error = ics18_result.expect_err("ICS18 error expected");
        assert!(matches!(
            ics18_error.kind(),
            ICS18ErrorKind::TransactionFailed
        ));
        let ics26_error = ics18_error
            .source()
            .expect("expected source in ICS18 error")
            .downcast_ref::<ICS26Error>()
            .expect("ICS18 source should be an ICS26 error");
        assert!(matches!(
            ics26_error.kind(),
            ICS26ErrorKind::HandlerRaisedError,
        ));
        ics26_error
            .source()
            .expect("expected source in ICS26 error")
            .downcast_ref::<anomaly::Error<K>>()
            .expect("ICS26 source should be an handler error")
            .kind()
            .clone()
    }

    // TODO: this is sometimes called version/revision number but seems
    //       unrelated with the `Version` type below; is that so?
    fn epoch() -> u64 {
        0
    }

    fn version() -> Version {
        Version::default()
    }

    fn versions() -> Vec<Version> {
        vec![Self::version()]
    }

    fn client_id(client_id: u64) -> ClientId {
        ClientId::new(ClientType::Mock, client_id)
            .expect("it should be possible to create the client identifier")
    }

    fn connection_id(connection_id: u64) -> ConnectionId {
        ConnectionId::new(connection_id)
            .expect("it should be possible to create the connection identifier")
    }

    fn height(height: u64) -> Height {
        Height::new(Self::epoch(), height)
    }

    fn mock_header(height: u64) -> MockHeader {
        MockHeader(Self::height(height))
    }

    fn header(height: u64) -> AnyHeader {
        AnyHeader::Mock(Self::mock_header(height))
    }

    fn client_state(height: u64) -> AnyClientState {
        AnyClientState::Mock(MockClientState(Self::mock_header(height)))
    }

    fn consensus_state(height: u64) -> AnyConsensusState {
        AnyConsensusState::Mock(MockConsensusState(Self::mock_header(height)))
    }

    fn signer() -> AccountId {
        AccountId::new([0; 20])
    }

    fn counterparty(counterparty_client_id: u64) -> Counterparty {
        let client_id = Self::client_id(counterparty_client_id);
        let connection_id = None;
        let prefix = Self::commitment_prefix();
        Counterparty::new(client_id, connection_id, prefix)
    }

    fn delay_period() -> u64 {
        0
    }

    fn commitment_prefix() -> CommitmentPrefix {
        CommitmentPrefix(Vec::new())
    }

    fn commitment_proof_bytes() -> CommitmentProofBytes {
        vec![0].into()
    }

    fn consensus_proof(height: u64) -> ConsensusProof {
        let consensus_proof = Self::commitment_proof_bytes();
        let consensus_height = Self::height(height);
        ConsensusProof::new(consensus_proof, consensus_height)
            .expect("it should be possible to create the consensus proof")
    }

    fn proofs(height: u64) -> Proofs {
        let object_proof = Self::commitment_proof_bytes();
        let client_proof = None;
        let consensus_proof = Some(Self::consensus_proof(height));
        let other_proof = None;
        let height = Self::height(height);
        Proofs::new(
            object_proof,
            client_proof,
            consensus_proof,
            other_proof,
            height,
        )
        .expect("it should be possible to create the proofs")
    }

    /// Check that chain heights match the ones in the model.
    fn check_chain_heights(&self, chains: HashMap<String, Chain>) -> bool {
        chains.into_iter().all(|(chain_id, chain)| {
            let ctx = self.chain_context(&chain_id);
            ctx.query_latest_height() == Self::height(chain.height)
        })
    }
}

impl modelator::TestExecutor<Step> for ICS02TestExecutor {
    fn initial_step(&mut self, step: Step) -> bool {
        assert_eq!(
            step.action.action_type,
            ActionType::None,
            "unexpected action type"
        );
        assert_eq!(
            step.action_outcome,
            ActionOutcome::None,
            "unexpected action outcome"
        );
        // initiliaze all chains
        for (chain_id, chain) in step.chains {
            self.init_chain_context(chain_id, chain.height);
        }
        true
    }

    fn next_step(&mut self, step: Step) -> bool {
        println!("{:?}", step);
        let outcome_matches = match step.action.action_type {
            ActionType::None => panic!("unexpected action type"),
            ActionType::ICS02CreateClient => {
                // get action parameters
                let chain_id = step
                    .action
                    .chain_id
                    .expect("create client action should have a chain identifier");
                let client_height = step
                    .action
                    .client_height
                    .expect("create client action should have a client height");

                // get chain's context
                let ctx = self.chain_context_mut(&chain_id);

                // create ICS26 message and deliver it
                let msg = ICS26Envelope::ICS2Msg(ClientMsg::CreateClient(MsgCreateAnyClient {
                    client_state: Self::client_state(client_height),
                    consensus_state: Self::consensus_state(client_height),
                    signer: Self::signer(),
                }));
                let result = ctx.deliver(msg);

                // check the expected outcome: client create always succeeds
                match step.action_outcome {
                    ActionOutcome::ICS02CreateOK => {
                        // the implementaion matches the model if no error occurs
                        result.is_ok()
                    }
                    action => panic!("unexpected action outcome {:?}", action),
                }
            }
            ActionType::ICS02UpdateClient => {
                // get action parameters
                let chain_id = step
                    .action
                    .chain_id
                    .expect("update client action should have a chain identifier");
                let client_id = step
                    .action
                    .client_id
                    .expect("update client action should have a client identifier");
                let client_height = step
                    .action
                    .client_height
                    .expect("update client action should have a client height");

                // get chain's context
                let ctx = self.chain_context_mut(&chain_id);

                // create ICS26 message and deliver it
                let msg = ICS26Envelope::ICS2Msg(ClientMsg::UpdateClient(MsgUpdateAnyClient {
                    client_id: Self::client_id(client_id),
                    header: Self::header(client_height),
                    signer: Self::signer(),
                }));
                let result = ctx.deliver(msg);

                // check the expected outcome
                match step.action_outcome {
                    ActionOutcome::ICS02UpdateOK => {
                        // the implementaion matches the model if no error occurs
                        result.is_ok()
                    }
                    ActionOutcome::ICS02ClientNotFound => {
                        let handler_error_kind =
                            Self::extract_handler_error_kind::<ICS02ErrorKind>(result);
                        // the implementaion matches the model if there's an
                        // error matching the expected outcome
                        matches!(
                            handler_error_kind,
                            ICS02ErrorKind::ClientNotFound(error_client_id)
                            if error_client_id == Self::client_id(client_id)
                        )
                    }
                    ActionOutcome::ICS02HeaderVerificationFailure => {
                        let handler_error_kind =
                            Self::extract_handler_error_kind::<ICS02ErrorKind>(result);
                        // the implementaion matches the model if there's an
                        // error matching the expected outcome
                        handler_error_kind == ICS02ErrorKind::HeaderVerificationFailure
                    }
                    action => panic!("unexpected action outcome {:?}", action),
                }
            }
            ActionType::ICS03ConnectionOpenInit => {
                // get action parameters
                let chain_id = step
                    .action
                    .chain_id
                    .expect("connection open init action should have a chain identifier");
                let client_id = step
                    .action
                    .client_id
                    .expect("connection open init action should have a client identifier");
                let counterparty_client_id = step.action.counterparty_client_id.expect(
                    "connection open init action should have a counterparty client identifier",
                );

                // get chain's context
                let ctx = self.chain_context_mut(&chain_id);

                // create ICS26 message and deliver it
                let msg = ICS26Envelope::ICS3Msg(ConnectionMsg::ConnectionOpenInit(
                    MsgConnectionOpenInit {
                        client_id: Self::client_id(client_id),
                        counterparty: Self::counterparty(counterparty_client_id),
                        version: Self::version(),
                        delay_period: Self::delay_period(),
                        signer: Self::signer(),
                    },
                ));
                let result = ctx.deliver(msg);

                // check the expected outcome
                match step.action_outcome {
                    ActionOutcome::ICS03ConnectionOpenInitOK => {
                        // the implementaion matches the model if no error occurs
                        result.is_ok()
                    }
                    ActionOutcome::ICS03MissingClient => {
                        let handler_error_kind =
                            Self::extract_handler_error_kind::<ICS03ErrorKind>(result);
                        // the implementaion matches the model if there's an
                        // error matching the expected outcome
                        matches!(
                            handler_error_kind,
                            ICS03ErrorKind::MissingClient(error_client_id)
                            if error_client_id == Self::client_id(client_id)
                        )
                    }
                    action => panic!("unexpected action outcome {:?}", action),
                }
            }
            ActionType::ICS03ConnectionOpenTry => {
                // get action parameters
                let chain_id = step
                    .action
                    .chain_id
                    .expect("connection open init action should have a chain identifier");
                let client_id = step
                    .action
                    .client_id
                    .expect("connection open init action should have a client identifier");
                let client_height = step
                    .action
                    .client_height
                    .expect("connection open try action should have a client height");
                let counterparty_client_id = step.action.counterparty_client_id.expect(
                    "connection open init action should have a counterparty client identifier",
                );
                let connection_id = step.action.connection_id;

                // get chain's context
                let ctx = self.chain_context_mut(&chain_id);

                // create ICS26 message and deliver it
                let msg = ICS26Envelope::ICS3Msg(ConnectionMsg::ConnectionOpenTry(Box::new(
                    MsgConnectionOpenTry {
                        previous_connection_id: connection_id.map(Self::connection_id),
                        client_id: Self::client_id(client_id),
                        client_state: None,
                        counterparty: Self::counterparty(counterparty_client_id),
                        counterparty_versions: Self::versions(),
                        proofs: Self::proofs(client_height),
                        delay_period: Self::delay_period(),
                        signer: Self::signer(),
                    },
                )));
                let result = ctx.deliver(msg);

                // check the expected outcome
                match step.action_outcome {
                    ActionOutcome::ICS03InvalidConsensusHeight => {
                        let handler_error_kind =
                            Self::extract_handler_error_kind::<ICS03ErrorKind>(result);
                        // the implementaion matches the model if there's an
                        // error matching the expected outcome
                        matches!(
                            handler_error_kind,
                            ICS03ErrorKind::InvalidConsensusHeight(error_consensus_height, _)
                            if error_consensus_height == Self::height(client_height)
                        )
                    }
                    ActionOutcome::ICS03ConnectionNotFound => {
                        let handler_error_kind =
                            Self::extract_handler_error_kind::<ICS03ErrorKind>(result);
                        // the implementaion matches the model if there's an
                        // error matching the expected outcome
                        connection_id.is_some()
                            && matches!(
                                handler_error_kind,
                                ICS03ErrorKind::ConnectionNotFound(error_connection_id)
                                if error_connection_id == Self::connection_id(connection_id.unwrap())
                            )
                    }
                    ActionOutcome::ICS03ConnectionMismatch => {
                        let handler_error_kind =
                            Self::extract_handler_error_kind::<ICS03ErrorKind>(result);
                        // the implementaion matches the model if there's an
                        // error matching the expected outcome
                        connection_id.is_some()
                            && matches!(
                                handler_error_kind,
                                ICS03ErrorKind::ConnectionMismatch(error_connection_id)
                                if error_connection_id == Self::connection_id(connection_id.unwrap())
                            )
                    }
                    action => panic!("unexpected action outcome {:?}", action),
                }
            }
        };
        // also check that chain heights match
        outcome_matches && self.check_chain_heights(step.chains)
    }
}

const TESTS_DIR: &str = "tests/support/model_based";

#[test]
fn main() {
    let tests = vec![
        "ICS02UpdateOKTest",
        "ICS02HeaderVerificationFailureTest",
        "ICS03ConnectionOpenInitOKTest",
        "ICS03MissingClientTest",
        "ICS03MissingClientTest",
        "ICS03InvalidConsensusHeightTest",
        "ICS03ConnectionNotFoundTest",
        "ICS03ConnectionMismatchTest",
    ];

    for test in tests {
        let path = format!("{}/{}.json", TESTS_DIR, test);
        let executor = ICS02TestExecutor::new();
        // we should be able to just return the `Result` once the following issue
        // is fixed: https://github.com/rust-lang/rust/issues/43301
        if let Err(e) = modelator::test_driver(executor, path) {
            panic!("{:?}", e);
        }
    }
}

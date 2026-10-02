use data_contracts::SystemDataContract;
use dpp::data_contract::serialized_version::DataContractInSerializationFormat;
use dpp::data_contract::{TokenConfiguration, TokenContractPosition};
use dpp::identifier::Identifier;
use dpp::platform_value::string_encoding::Encoding::Base58;
use dpp::state_transition::data_contract_create_transition::DataContractCreateTransition;
use dpp::state_transition::data_contract_update_transition::DataContractUpdateTransition;
use serde_json::Value;
use std::collections::BTreeMap;
use tokio_postgres::Row;

#[derive(Clone)]
pub struct DataContract {
    pub id: Option<u32>,
    pub name: Option<String>,
    pub owner: Identifier,
    pub identifier: Identifier,
    pub schema: Option<Value>,
    pub version: u32,
    pub is_system: bool,
    pub tokens: Option<BTreeMap<TokenContractPosition, TokenConfiguration>>,
    pub format_version: Option<u32>,
    pub description: Option<String>,
    pub keywords: Vec<String>,
}

impl From<DataContractCreateTransition> for DataContract {
    fn from(state_transition: DataContractCreateTransition) -> Self {
        let data_contract = match state_transition {
            DataContractCreateTransition::V0(st) => st.data_contract,
            DataContractCreateTransition::V1(st) => st.data_contract,
        };
        match data_contract {
            DataContractInSerializationFormat::V0(data_contract) => {
                let identifier = data_contract.id;
                let version = data_contract.version;
                let owner = data_contract.owner_id;
                let schema = data_contract.document_schemas;
                let schema_decoded = serde_json::to_value(schema).unwrap();

                return DataContract {
                    id: None,
                    name: None,
                    owner,
                    identifier,
                    schema: Some(schema_decoded),
                    version,
                    is_system: false,
                    tokens: None,
                    format_version: Some(0u32),
                    description: None,
                    keywords: vec![],
                };
            }

            DataContractInSerializationFormat::V1(data_contract) => {
                let identifier = data_contract.id;
                let version = data_contract.version;
                let owner = data_contract.owner_id;
                let schema = data_contract.document_schemas;
                let schema_decoded = serde_json::to_value(schema).unwrap();
                let tokens = data_contract.tokens;
                let description = data_contract.description;
                let keywords = data_contract.keywords;

                return DataContract {
                    id: None,
                    name: None,
                    owner,
                    identifier,
                    schema: Some(schema_decoded),
                    version,
                    is_system: false,
                    tokens: Some(tokens),
                    format_version: Some(1u32),
                    description,
                    keywords,
                };
            }
        }
    }
}

impl From<DataContractUpdateTransition> for DataContract {
    fn from(state_transition: DataContractUpdateTransition) -> Self {
        match state_transition {
            DataContractUpdateTransition::V0(data_contract_update_transition) => {
                let data_contract = data_contract_update_transition.data_contract;

                match data_contract {
                    DataContractInSerializationFormat::V0(data_contract) => {
                        let identifier = data_contract.id;
                        let version = data_contract.version;
                        let owner = data_contract.owner_id;
                        let schema = data_contract.document_schemas;
                        let schema_decoded = serde_json::to_value(schema).unwrap();

                        return DataContract {
                            id: None,
                            name: None,
                            owner,
                            identifier,
                            schema: Some(schema_decoded),
                            version,
                            is_system: false,
                            tokens: None,
                            format_version: Some(0u32),
                            description: None,
                            keywords: vec![],
                        };
                    }

                    DataContractInSerializationFormat::V1(data_contract) => {
                        let identifier = data_contract.id;
                        let version = data_contract.version;
                        let owner = data_contract.owner_id;
                        let schema = data_contract.document_schemas;
                        let schema_decoded = serde_json::to_value(schema).unwrap();
                        let tokens = data_contract.tokens;
                        let description = data_contract.description;
                        let keywords = data_contract.keywords;

                        return DataContract {
                            id: None,
                            name: None,
                            owner,
                            identifier,
                            schema: Some(schema_decoded),
                            version,
                            is_system: false,
                            tokens: Some(tokens),
                            format_version: Some(1u32),
                            description,
                            keywords,
                        };
                    }
                }
            }
        }
    }
}

impl From<SystemDataContract> for DataContract {
    fn from(data_contract: SystemDataContract) -> Self {
        let platform_version = dpp::version::PlatformVersion::get(14)
            .expect("protocol 14 must be supported by pinned DPP");
        let name = match data_contract {
            SystemDataContract::Withdrawals => "Withdrawals",
            SystemDataContract::MasternodeRewards => "MasternodeRewards",
            SystemDataContract::FeatureFlags => "FeatureFlags",
            SystemDataContract::DPNS => "DPNS",
            SystemDataContract::Dashpay => "Dashpay",
            SystemDataContract::WalletUtils => "WalletUtils",
            SystemDataContract::TokenHistory => "TokenHistory",
            SystemDataContract::KeywordSearch => "KeywordSearch",
            SystemDataContract::DocumentHistory => "DocumentHistory",
            SystemDataContract::AppConnect => "AppConnect",
            SystemDataContract::ModerationCharters => "ModerationCharters",
        };
        let identifier = data_contract.id();
        let source = data_contract.source(platform_version).ok();
        let owner = source
            .as_ref()
            .map(|s| Identifier::from(s.owner_id_bytes))
            .unwrap_or(identifier);
        let source_version = source.as_ref().map(|source| source.version).unwrap_or(0);
        let schema_decoded = source.map(|s| serde_json::to_value(s.document_schemas).unwrap());

        DataContract {
            id: None,
            name: Some(String::from(name)),
            owner,
            identifier,
            schema: schema_decoded,
            version: source_version,
            is_system: true,
            tokens: None,
            format_version: None,
            description: None,
            keywords: vec![],
        }
    }
}

impl From<Row> for DataContract {
    fn from(row: Row) -> Self {
        let id: i32 = row.get(0);
        let name: Option<String> = row.get(1);
        let owner: String = row.get(2);
        let identifier: String = row.get(3);
        let version: i32 = row.get(4);
        let is_system: bool = row.get(5);

        return DataContract {
            id: Some(id as u32),
            name,
            owner: Identifier::from_string(owner.trim(), Base58).unwrap(),
            identifier: Identifier::from_string(identifier.trim(), Base58).unwrap(),
            schema: None,
            version: version as u32,
            is_system,
            tokens: None,
            format_version: None,
            description: None,
            keywords: vec![],
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_14_catalog_has_real_schema_and_owner_for_every_registered_contract() {
        let version = dpp::version::PlatformVersion::get(14).unwrap();
        let mut count = 0;
        for contract in SystemDataContract::ALL {
            if contract == SystemDataContract::FeatureFlags {
                continue;
            }
            let source = contract.source(version).unwrap();
            let indexed = DataContract::from(contract);
            assert_eq!(indexed.version, source.version);
            assert_eq!(indexed.owner, Identifier::from(source.owner_id_bytes));
            assert_eq!(indexed.schema, Some(source.document_schemas));
            count += 1;
        }
        assert_eq!(count, 10);
    }
}

// Copyright 2018 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::client::Client;
use crate::crypto::shared_secretbox;
use crate::event_loop::CoreFuture;
use crate::self_encryption_storage::SelfEncryptionStorage;
use crate::utils::{self, FutureExt};
use futures::Future;
use maidsafe_utilities::serialisation::{deserialise, serialise};
use safe_nd::{IData, IDataAddress, PubImmutableData, UnpubImmutableData};
use self_encryption::{DataMap, SelfEncryptor};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
enum DataTypeEncoding {
    Serialised(Vec<u8>),
    DataMap(DataMap),
}

/// Create and obtain immutable data out of the given raw bytes. This will encrypt the right content
/// if the keys are provided and will ensure the maximum immutable data chunk size is respected.
pub fn create(
    client: &impl Client,
    value: &[u8],
    published: bool,
    encryption_key: Option<shared_secretbox::Key>,
) -> Box<CoreFuture<IData>> {
    trace!("Creating conformant ImmutableData.");

    let client = client.clone();
    let storage = SelfEncryptionStorage::new(client.clone(), published);
    let self_encryptor = fry!(SelfEncryptor::new(storage, DataMap::None));

    self_encryptor
        .write(value, 0)
        .and_then(move |_| self_encryptor.close())
        .map_err(From::from)
        .and_then(move |(data_map, _)| {
            let serialised_data_map = fry!(serialise(&data_map));

            let value = if let Some(key) = encryption_key {
                let cipher_text = fry!(utils::symmetric_encrypt(&serialised_data_map, &key, None));
                fry!(serialise(&DataTypeEncoding::Serialised(cipher_text)))
            } else {
                fry!(serialise(&DataTypeEncoding::Serialised(
                    serialised_data_map
                ),))
            };

            pack(client, value, published)
        })
        .into_box()
}

/// Get the raw bytes from `ImmutableData` created via the `create` function in this module.
pub fn extract_value(
    client: &impl Client,
    data: &IData,
    decryption_key: Option<shared_secretbox::Key>,
) -> Box<CoreFuture<Vec<u8>>> {
    let client = client.clone();
    let published = data.is_pub();
    unpack(client.clone(), data)
        .and_then(move |value| {
            let data_map = if let Some(key) = decryption_key {
                let plain_text = utils::symmetric_decrypt(&value, &key)?;
                deserialise(&plain_text)?
            } else {
                deserialise(&value)?
            };

            let storage = SelfEncryptionStorage::new(client, published);
            Ok(SelfEncryptor::new(storage, data_map)?)
        })
        .and_then(|self_encryptor| {
            let length = self_encryptor.len();
            self_encryptor.read(0, length).map_err(From::from)
        })
        .into_box()
}

/// Get immutable data from the network and extract its value, decrypting it in the process (if keys
/// provided). This combines `get_idata` in `Client` and `extract_value` in this module into one
/// function.
pub fn get_value(
    client: &impl Client,
    address: IDataAddress,
    decryption_key: Option<shared_secretbox::Key>,
) -> Box<CoreFuture<Vec<u8>>> {
    let client2 = client.clone();
    client
        .get_idata(address)
        .and_then(move |data| extract_value(&client2, &data, decryption_key))
        .into_box()
}

// TODO: consider rewriting these two function to not use recursion.

fn pack(client: impl Client, value: Vec<u8>, published: bool) -> Box<CoreFuture<IData>> {
    let data: IData = if published {
        PubImmutableData::new(value).into()
    } else {
        UnpubImmutableData::new(value, client.public_key()).into()
    };
    let serialised_data = fry!(serialise(&data));

    if !data.validate_size() {
        let storage = SelfEncryptionStorage::new(client.clone(), published);
        let self_encryptor = fry!(SelfEncryptor::new(storage, DataMap::None));
        self_encryptor
            .write(&serialised_data, 0)
            .and_then(move |_| self_encryptor.close())
            .map_err(From::from)
            .and_then(move |(data_map, _)| {
                let value = fry!(serialise(&DataTypeEncoding::DataMap(data_map)));
                pack(client, value, published)
            })
            .into_box()
    } else {
        ok!(data)
    }
}

fn unpack(client: impl Client, data: &IData) -> Box<CoreFuture<Vec<u8>>> {
    let published = data.is_pub();
    match fry!(deserialise(data.value())) {
        DataTypeEncoding::Serialised(value) => ok!(value),
        DataTypeEncoding::DataMap(data_map) => {
            let storage = SelfEncryptionStorage::new(client.clone(), published);
            let self_encryptor = fry!(SelfEncryptor::new(storage, data_map));
            let length = self_encryptor.len();
            self_encryptor
                .read(0, length)
                .map_err(From::from)
                .and_then(move |serialised_data| {
                    let data = fry!(deserialise(&serialised_data));
                    unpack(client, &data)
                })
                .into_box()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::Future;
    use utils;
    use utils::test_utils::{finish, random_client};

    // Test creating and retrieving a 1kb idata.
    #[test]
    fn create_and_retrieve_1kb() {
        create_and_retrieve(1024)
    }

    // Test creating and retrieving a 1mb idata.
    #[test]
    fn create_and_retrieve_1mb() {
        create_and_retrieve(1024 * 1024)
    }

    // Test creating and retrieving a 2mb idata.
    #[test]
    fn create_and_retrieve_2mb() {
        create_and_retrieve(2 * 1024 * 1024)
    }

    // Test creating and retrieving a 10mb idata.
    #[cfg(not(debug_assertions))]
    #[test]
    fn create_and_retrieve_10mb() {
        create_and_retrieve(10 * 1024 * 1024)
    }

    fn create_and_retrieve(size: usize) {
        let value = unwrap!(utils::generate_random_vector(size));

        // Unencrypted and published
        {
            let value_before = value.clone();

            random_client(move |client| {
                let client2 = client.clone();
                let client3 = client.clone();

                create(client, &value_before.clone(), true, None)
                    .then(move |res| {
                        let data_before = unwrap!(res);
                        let address = *data_before.address();
                        client2.put_idata(data_before).map(move |_| address)
                    })
                    .then(move |res| {
                        let address = unwrap!(res);
                        get_value(&client3, address, None)
                    })
                    .then(move |res| {
                        let value_after = unwrap!(res);
                        assert_eq!(value_after, value_before);
                        finish()
                    })
            })
        }

        // Encrypted and unpublished
        {
            let value_before = value.clone();
            let key = shared_secretbox::gen_key();

            random_client(move |client| {
                let client2 = client.clone();
                let client3 = client.clone();

                create(client, &value_before.clone(), false, Some(key.clone()))
                    .then(move |res| {
                        let data_before = unwrap!(res);
                        let address = *data_before.address();
                        client2.put_idata(data_before).map(move |_| address)
                    })
                    .then(move |res| {
                        let address = unwrap!(res);
                        get_value(&client3, address, Some(key))
                    })
                    .then(move |res| {
                        let value_after = unwrap!(res);
                        assert_eq!(value_after, value_before);
                        finish()
                    })
            })
        }

        // Put unencrypted Retrieve encrypted - Should fail
        {
            let value = value.clone();
            let key = shared_secretbox::gen_key();

            random_client(move |client| {
                let client2 = client.clone();
                let client3 = client.clone();

                create(client, &value, true, None)
                    .then(move |res| {
                        let data = unwrap!(res);
                        let address = *data.address();
                        client2.put_idata(data).map(move |_| address)
                    })
                    .then(move |res| {
                        let address = unwrap!(res);
                        get_value(&client3, address, Some(key))
                    })
                    .then(|res| {
                        assert!(res.is_err());
                        finish()
                    })
            })
        }

        // Put encrypted Retrieve unencrypted - Should fail
        {
            let value = value.clone();
            let key = shared_secretbox::gen_key();

            random_client(move |client| {
                let client2 = client.clone();
                let client3 = client.clone();

                create(client, &value, true, Some(key))
                    .then(move |res| {
                        let data = unwrap!(res);
                        let address = *data.address();
                        client2.put_idata(data).map(move |_| address)
                    })
                    .then(move |res| {
                        let address = unwrap!(res);
                        get_value(&client3, address, None)
                    })
                    .then(|res| {
                        assert!(res.is_err());
                        finish()
                    })
            })
        }

        // Put published Retrieve unpublished - Should fail
        {
            let value = value.clone();

            random_client(move |client| {
                let client2 = client.clone();
                let client3 = client.clone();

                create(client, &value, true, None)
                    .then(move |res| {
                        let data = unwrap!(res);
                        let data_name = *data.name();
                        client2.put_idata(data).map(move |_| data_name)
                    })
                    .then(move |res| {
                        let data_name = unwrap!(res);
                        let address = IDataAddress::Unpub(data_name);
                        get_value(&client3, address, None)
                    })
                    .then(|res| {
                        assert!(res.is_err());
                        finish()
                    })
            })
        }

        // Put unpublished Retrieve published - Should fail
        {
            let value = value.clone();

            random_client(move |client| {
                let client2 = client.clone();
                let client3 = client.clone();

                create(client, &value, false, None)
                    .then(move |res| {
                        let data = unwrap!(res);
                        let data_name = *data.name();
                        client2.put_idata(data).map(move |_| data_name)
                    })
                    .then(move |res| {
                        let data_name = unwrap!(res);
                        let address = IDataAddress::Pub(data_name);
                        get_value(&client3, address, None)
                    })
                    .then(|res| {
                        assert!(res.is_err());
                        finish()
                    })
            })
        }
    }
}

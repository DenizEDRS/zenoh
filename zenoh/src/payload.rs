//
// Copyright (c) 2023 ZettaScale Technology
//
// This program and the accompanying materials are made available under the
// terms of the Eclipse Public License 2.0 which is available at
// http://www.eclipse.org/legal/epl-2.0, or the Apache License, Version 2.0
// which is available at https://www.apache.org/licenses/LICENSE-2.0.
//
// SPDX-License-Identifier: EPL-2.0 OR Apache-2.0
//
// Contributors:
//   ZettaScale Zenoh Team, <zenoh@zettascale.tech>
//

//! Payload primitives.
use crate::buffers::ZBuf;
use std::marker::PhantomData;
use std::{
    borrow::Cow, convert::Infallible, fmt::Debug, ops::Deref, string::FromUtf8Error, sync::Arc,
};
use unwrap_infallible::UnwrapInfallible;
use zenoh_buffers::{
    buffer::{Buffer, SplitBuffer},
    reader::{HasReader, Reader},
    writer::HasWriter,
    ZBufReader, ZSlice,
};
use zenoh_codec::{RCodec, WCodec, Zenoh080};
use zenoh_result::{ZError, ZResult};
#[cfg(feature = "shared-memory")]
use zenoh_shm::SharedMemoryBuf;

/// Trait to encode a type `T` into a [`Value`].
pub trait Serialize<T> {
    type Output;

    /// The implementer should take care of serializing the type `T` and set the proper [`Encoding`].
    fn serialize(self, t: T) -> Self::Output;
}

pub trait Deserialize<'a, T> {
    type Error;

    /// The implementer should take care of deserializing the type `T` based on the [`Encoding`] information.
    fn deserialize(self, t: &'a Payload) -> Result<T, Self::Error>;
}

/// A payload contains the serialized bytes of user data.
#[repr(transparent)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Payload(ZBuf);

impl Payload {
    /// Create an empty payload.
    pub const fn empty() -> Self {
        Self(ZBuf::empty())
    }

    /// Create a [`Payload`] from any type `T` that can implements [`Into<ZBuf>`].
    pub fn new<T>(t: T) -> Self
    where
        T: Into<ZBuf>,
    {
        Self(t.into())
    }

    /// Returns wether the payload is empty or not.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the length of the payload.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Get a [`PayloadReader`] implementing [`std::io::Read`] trait.
    pub fn reader(&self) -> PayloadReader<'_> {
        PayloadReader(self.0.reader())
    }

    /// Get a [`PayloadReader`] implementing [`std::io::Read`] trait.
    pub fn iter<T>(&self) -> PayloadIterator<'_, T>
    where
        T: TryFrom<Payload>,
        ZSerde: for<'b> Deserialize<'b, T, Error = ZDeserializeError>,
    {
        PayloadIterator {
            reader: self.0.reader(),
            _t: PhantomData::<T>,
        }
    }
}

/// Provide some facilities specific to the Rust API to encode/decode a [`Value`] with an `Serialize`.
impl Payload {
    /// Encode an object of type `T` as a [`Value`] using the [`ZSerde`].
    ///
    /// ```rust
    /// use zenoh::payload::Payload;
    ///
    /// let start = String::from("abc");
    /// let payload = Payload::serialize(start.clone());
    /// let end: String = payload.deserialize().unwrap();
    /// assert_eq!(start, end);
    /// ```
    pub fn serialize<T>(t: T) -> Self
    where
        ZSerde: Serialize<T, Output = Payload>,
    {
        ZSerde.serialize(t)
    }

    /// Decode an object of type `T` from a [`Value`] using the [`ZSerde`].
    /// See [encode](Value::encode) for an example.
    pub fn deserialize<'a, T>(&'a self) -> ZResult<T>
    where
        ZSerde: Deserialize<'a, T>,
        <ZSerde as Deserialize<'a, T>>::Error: Debug,
    {
        let t: T = ZSerde.deserialize(self).map_err(|e| zerror!("{:?}", e))?;
        Ok(t)
    }
}

/// A reader that implements [`std::io::Read`] trait to read from a [`Payload`].
pub struct PayloadReader<'a>(ZBufReader<'a>);

impl std::io::Read for PayloadReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        std::io::Read::read(&mut self.0, buf)
    }
}

/// An iterator that implements [`std::iter::Iterator`] trait to iterate on values `T` in a [`Payload`].
/// Note that [`Payload`] contains a serialized version of `T` and iterating over a [`Payload`] performs lazy deserialization.
pub struct PayloadIterator<'a, T>
where
    ZSerde: Deserialize<'a, T>,
{
    reader: ZBufReader<'a>,
    _t: PhantomData<T>,
}

impl<'a, T> Iterator for PayloadIterator<'a, T>
where
    ZSerde: for<'b> Deserialize<'b, T, Error = ZDeserializeError>,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        let codec = Zenoh080::new();

        let kbuf: ZBuf = codec.read(&mut self.reader).ok()?;
        let kpld = Payload::new(kbuf);

        let t = ZSerde.deserialize(&kpld).ok()?;
        Some(t)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.reader.remaining();
        (remaining, Some(remaining))
    }
}

/// The default serializer for Zenoh payload. It supports primitives types, such as: vec<u8>, int, uint, float, string, bool.
/// It also supports common Rust serde values.
#[derive(Clone, Copy, Debug)]
pub struct ZSerde;

#[derive(Debug, Clone, Copy)]
pub struct ZDeserializeError;

// ZBuf
impl Serialize<ZBuf> for ZSerde {
    type Output = Payload;

    fn serialize(self, t: ZBuf) -> Self::Output {
        Payload::new(t)
    }
}

impl From<ZBuf> for Payload {
    fn from(t: ZBuf) -> Self {
        ZSerde.serialize(t)
    }
}

impl Deserialize<'_, ZBuf> for ZSerde {
    type Error = Infallible;

    fn deserialize(self, v: &Payload) -> Result<ZBuf, Self::Error> {
        Ok(v.0.clone())
    }
}

impl From<Payload> for ZBuf {
    fn from(value: Payload) -> Self {
        value.0
    }
}

impl From<&Payload> for ZBuf {
    fn from(value: &Payload) -> Self {
        ZSerde.deserialize(value).unwrap_infallible()
    }
}

// Vec<u8>
impl Serialize<Vec<u8>> for ZSerde {
    type Output = Payload;

    fn serialize(self, t: Vec<u8>) -> Self::Output {
        Payload::new(t)
    }
}

impl From<Vec<u8>> for Payload {
    fn from(t: Vec<u8>) -> Self {
        ZSerde.serialize(t)
    }
}

impl Deserialize<'_, Vec<u8>> for ZSerde {
    type Error = Infallible;

    fn deserialize(self, v: &Payload) -> Result<Vec<u8>, Self::Error> {
        Ok(v.0.contiguous().to_vec())
    }
}

impl From<Payload> for Vec<u8> {
    fn from(value: Payload) -> Self {
        ZSerde.deserialize(&value).unwrap_infallible()
    }
}

impl From<&Payload> for Vec<u8> {
    fn from(value: &Payload) -> Self {
        ZSerde.deserialize(value).unwrap_infallible()
    }
}

// &[u8]
impl Serialize<&[u8]> for ZSerde {
    type Output = Payload;

    fn serialize(self, t: &[u8]) -> Self::Output {
        Payload::new(t.to_vec())
    }
}

impl From<&[u8]> for Payload {
    fn from(t: &[u8]) -> Self {
        ZSerde.serialize(t)
    }
}

// Cow<[u8]>
impl<'a> Serialize<Cow<'a, [u8]>> for ZSerde {
    type Output = Payload;

    fn serialize(self, t: Cow<'a, [u8]>) -> Self::Output {
        Payload::new(t.to_vec())
    }
}

impl From<Cow<'_, [u8]>> for Payload {
    fn from(t: Cow<'_, [u8]>) -> Self {
        ZSerde.serialize(t)
    }
}

impl<'a> Deserialize<'a, Cow<'a, [u8]>> for ZSerde {
    type Error = Infallible;

    fn deserialize(self, v: &'a Payload) -> Result<Cow<'a, [u8]>, Self::Error> {
        Ok(Cow::from(v))
    }
}

impl<'a> From<&'a Payload> for Cow<'a, [u8]> {
    fn from(value: &'a Payload) -> Self {
        ZSerde.deserialize(value).unwrap_infallible()
    }
}

// String
impl Serialize<String> for ZSerde {
    type Output = Payload;

    fn serialize(self, s: String) -> Self::Output {
        Payload::new(s.into_bytes())
    }
}

impl From<String> for Payload {
    fn from(t: String) -> Self {
        ZSerde.serialize(t)
    }
}

impl Deserialize<'_, String> for ZSerde {
    type Error = FromUtf8Error;

    fn deserialize(self, v: &Payload) -> Result<String, Self::Error> {
        let v: Vec<u8> = ZSerde.deserialize(v).unwrap_infallible();
        String::from_utf8(v)
    }
}

impl TryFrom<Payload> for String {
    type Error = FromUtf8Error;

    fn try_from(value: Payload) -> Result<Self, Self::Error> {
        ZSerde.deserialize(&value)
    }
}

impl TryFrom<&Payload> for String {
    type Error = FromUtf8Error;

    fn try_from(value: &Payload) -> Result<Self, Self::Error> {
        ZSerde.deserialize(value)
    }
}

// &str
impl Serialize<&str> for ZSerde {
    type Output = Payload;

    fn serialize(self, s: &str) -> Self::Output {
        Self.serialize(s.to_string())
    }
}

impl From<&str> for Payload {
    fn from(t: &str) -> Self {
        ZSerde.serialize(t)
    }
}

// Cow<str>
impl<'a> Serialize<Cow<'a, str>> for ZSerde {
    type Output = Payload;

    fn serialize(self, s: Cow<'a, str>) -> Self::Output {
        Self.serialize(s.to_string())
    }
}

impl From<Cow<'_, str>> for Payload {
    fn from(t: Cow<'_, str>) -> Self {
        ZSerde.serialize(t)
    }
}

impl<'a> Deserialize<'a, Cow<'a, str>> for ZSerde {
    type Error = FromUtf8Error;

    fn deserialize(self, v: &Payload) -> Result<Cow<'a, str>, Self::Error> {
        let v: String = Self.deserialize(v)?;
        Ok(Cow::Owned(v))
    }
}

impl<'a> TryFrom<&'a Payload> for Cow<'a, str> {
    type Error = FromUtf8Error;

    fn try_from(value: &'a Payload) -> Result<Self, Self::Error> {
        ZSerde.deserialize(value)
    }
}

// - Integers impl
macro_rules! impl_int {
    ($t:ty, $encoding:expr) => {
        impl Serialize<$t> for ZSerde {
            type Output = Payload;

            fn serialize(self, t: $t) -> Self::Output {
                let bs = t.to_le_bytes();
                let end = if t == 0 as $t {
                    0
                } else {
                    1 + bs.iter().rposition(|b| *b != 0).unwrap_or(bs.len() - 1)
                };
                // SAFETY:
                // - 0 is a valid start index because bs is guaranteed to always have a length greater or equal than 1
                // - end is a valid end index because is bounded between 0 and bs.len()
                Payload::new(unsafe { ZSlice::new_unchecked(Arc::new(bs), 0, end) })
            }
        }

        impl From<$t> for Payload {
            fn from(t: $t) -> Self {
                ZSerde.serialize(t)
            }
        }

        impl Serialize<&$t> for ZSerde {
            type Output = Payload;

            fn serialize(self, t: &$t) -> Self::Output {
                Self.serialize(*t)
            }
        }

        impl From<&$t> for Payload {
            fn from(t: &$t) -> Self {
                ZSerde.serialize(t)
            }
        }

        impl Serialize<&mut $t> for ZSerde {
            type Output = Payload;

            fn serialize(self, t: &mut $t) -> Self::Output {
                ZSerde.serialize(*t)
            }
        }

        impl From<&mut $t> for Payload {
            fn from(t: &mut $t) -> Self {
                ZSerde.serialize(t)
            }
        }

        impl<'a> Deserialize<'a, $t> for ZSerde {
            type Error = ZDeserializeError;

            fn deserialize(self, v: &Payload) -> Result<$t, Self::Error> {
                use std::io::Read;

                let mut r = v.reader();
                let mut bs = (0 as $t).to_le_bytes();
                if v.len() > bs.len() {
                    return Err(ZDeserializeError);
                }
                r.read_exact(&mut bs[..v.len()])
                    .map_err(|_| ZDeserializeError)?;
                let t = <$t>::from_le_bytes(bs);
                Ok(t)
            }
        }

        impl TryFrom<Payload> for $t {
            type Error = ZDeserializeError;

            fn try_from(value: Payload) -> Result<Self, Self::Error> {
                ZSerde.deserialize(&value)
            }
        }

        impl TryFrom<&Payload> for $t {
            type Error = ZDeserializeError;

            fn try_from(value: &Payload) -> Result<Self, Self::Error> {
                ZSerde.deserialize(value)
            }
        }
    };
}

// Zenoh unsigned integers
impl_int!(u8, ZSerde::ZENOH_UINT);
impl_int!(u16, ZSerde::ZENOH_UINT);
impl_int!(u32, ZSerde::ZENOH_UINT);
impl_int!(u64, ZSerde::ZENOH_UINT);
impl_int!(usize, ZSerde::ZENOH_UINT);

// Zenoh signed integers
impl_int!(i8, ZSerde::ZENOH_INT);
impl_int!(i16, ZSerde::ZENOH_INT);
impl_int!(i32, ZSerde::ZENOH_INT);
impl_int!(i64, ZSerde::ZENOH_INT);
impl_int!(isize, ZSerde::ZENOH_INT);

// Zenoh floats
impl_int!(f32, ZSerde::ZENOH_FLOAT);
impl_int!(f64, ZSerde::ZENOH_FLOAT);

// Zenoh bool
impl Serialize<bool> for ZSerde {
    type Output = Payload;

    fn serialize(self, t: bool) -> Self::Output {
        // SAFETY: casting a bool into an integer is well-defined behaviour.
        //      0 is false, 1 is true: https://doc.rust-lang.org/std/primitive.bool.html
        Payload::new(ZBuf::from((t as u8).to_le_bytes()))
    }
}

impl From<bool> for Payload {
    fn from(t: bool) -> Self {
        ZSerde.serialize(t)
    }
}

impl Deserialize<'_, bool> for ZSerde {
    type Error = ZDeserializeError;

    fn deserialize(self, v: &Payload) -> Result<bool, Self::Error> {
        let p = v.deserialize::<u8>().map_err(|_| ZDeserializeError)?;
        match p {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ZDeserializeError),
        }
    }
}

impl TryFrom<&Payload> for bool {
    type Error = ZDeserializeError;

    fn try_from(value: &Payload) -> Result<Self, Self::Error> {
        ZSerde.deserialize(value)
    }
}

// - Zenoh advanced types encoders/decoders
// JSON
impl Serialize<&serde_json::Value> for ZSerde {
    type Output = Result<Payload, serde_json::Error>;

    fn serialize(self, t: &serde_json::Value) -> Self::Output {
        let mut payload = Payload::empty();
        serde_json::to_writer(payload.0.writer(), t)?;
        Ok(payload)
    }
}

impl TryFrom<&serde_json::Value> for Payload {
    type Error = serde_json::Error;

    fn try_from(value: &serde_json::Value) -> Result<Self, Self::Error> {
        ZSerde.serialize(value)
    }
}

impl Serialize<serde_json::Value> for ZSerde {
    type Output = Result<Payload, serde_json::Error>;

    fn serialize(self, t: serde_json::Value) -> Self::Output {
        Self.serialize(&t)
    }
}

impl TryFrom<serde_json::Value> for Payload {
    type Error = serde_json::Error;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        ZSerde.serialize(value)
    }
}

impl Deserialize<'_, serde_json::Value> for ZSerde {
    type Error = serde_json::Error;

    fn deserialize(self, v: &Payload) -> Result<serde_json::Value, Self::Error> {
        serde_json::from_reader(v.reader())
    }
}

impl TryFrom<&Payload> for serde_json::Value {
    type Error = serde_json::Error;

    fn try_from(value: &Payload) -> Result<Self, Self::Error> {
        ZSerde.deserialize(value)
    }
}

// Yaml
impl Serialize<&serde_yaml::Value> for ZSerde {
    type Output = Result<Payload, serde_yaml::Error>;

    fn serialize(self, t: &serde_yaml::Value) -> Self::Output {
        let mut payload = Payload::empty();
        serde_yaml::to_writer(payload.0.writer(), t)?;
        Ok(payload)
    }
}

impl TryFrom<&serde_yaml::Value> for Payload {
    type Error = serde_yaml::Error;

    fn try_from(value: &serde_yaml::Value) -> Result<Self, Self::Error> {
        ZSerde.serialize(value)
    }
}

impl Serialize<serde_yaml::Value> for ZSerde {
    type Output = Result<Payload, serde_yaml::Error>;

    fn serialize(self, t: serde_yaml::Value) -> Self::Output {
        Self.serialize(&t)
    }
}

impl TryFrom<serde_yaml::Value> for Payload {
    type Error = serde_yaml::Error;

    fn try_from(value: serde_yaml::Value) -> Result<Self, Self::Error> {
        ZSerde.serialize(value)
    }
}

impl Deserialize<'_, serde_yaml::Value> for ZSerde {
    type Error = serde_yaml::Error;

    fn deserialize(self, v: &Payload) -> Result<serde_yaml::Value, Self::Error> {
        serde_yaml::from_reader(v.reader())
    }
}

impl TryFrom<&Payload> for serde_yaml::Value {
    type Error = serde_yaml::Error;

    fn try_from(value: &Payload) -> Result<Self, Self::Error> {
        ZSerde.deserialize(value)
    }
}

// CBOR
impl Serialize<&serde_cbor::Value> for ZSerde {
    type Output = Result<Payload, serde_cbor::Error>;

    fn serialize(self, t: &serde_cbor::Value) -> Self::Output {
        let mut payload = Payload::empty();
        serde_cbor::to_writer(payload.0.writer(), t)?;
        Ok(payload)
    }
}

impl TryFrom<&serde_cbor::Value> for Payload {
    type Error = serde_cbor::Error;

    fn try_from(value: &serde_cbor::Value) -> Result<Self, Self::Error> {
        ZSerde.serialize(value)
    }
}

impl Serialize<serde_cbor::Value> for ZSerde {
    type Output = Result<Payload, serde_cbor::Error>;

    fn serialize(self, t: serde_cbor::Value) -> Self::Output {
        Self.serialize(&t)
    }
}

impl TryFrom<serde_cbor::Value> for Payload {
    type Error = serde_cbor::Error;

    fn try_from(value: serde_cbor::Value) -> Result<Self, Self::Error> {
        ZSerde.serialize(value)
    }
}

impl Deserialize<'_, serde_cbor::Value> for ZSerde {
    type Error = serde_cbor::Error;

    fn deserialize(self, v: &Payload) -> Result<serde_cbor::Value, Self::Error> {
        serde_cbor::from_reader(v.reader())
    }
}

impl TryFrom<&Payload> for serde_cbor::Value {
    type Error = serde_cbor::Error;

    fn try_from(value: &Payload) -> Result<Self, Self::Error> {
        ZSerde.deserialize(value)
    }
}

// Pickle
impl Serialize<&serde_pickle::Value> for ZSerde {
    type Output = Result<Payload, serde_pickle::Error>;

    fn serialize(self, t: &serde_pickle::Value) -> Self::Output {
        let mut payload = Payload::empty();
        serde_pickle::value_to_writer(
            &mut payload.0.writer(),
            t,
            serde_pickle::SerOptions::default(),
        )?;
        Ok(payload)
    }
}

impl TryFrom<&serde_pickle::Value> for Payload {
    type Error = serde_pickle::Error;

    fn try_from(value: &serde_pickle::Value) -> Result<Self, Self::Error> {
        ZSerde.serialize(value)
    }
}

impl Serialize<serde_pickle::Value> for ZSerde {
    type Output = Result<Payload, serde_pickle::Error>;

    fn serialize(self, t: serde_pickle::Value) -> Self::Output {
        Self.serialize(&t)
    }
}

impl TryFrom<serde_pickle::Value> for Payload {
    type Error = serde_pickle::Error;

    fn try_from(value: serde_pickle::Value) -> Result<Self, Self::Error> {
        ZSerde.serialize(value)
    }
}

impl Deserialize<'_, serde_pickle::Value> for ZSerde {
    type Error = serde_pickle::Error;

    fn deserialize(self, v: &Payload) -> Result<serde_pickle::Value, Self::Error> {
        serde_pickle::value_from_reader(v.reader(), serde_pickle::DeOptions::default())
    }
}

impl TryFrom<&Payload> for serde_pickle::Value {
    type Error = serde_pickle::Error;

    fn try_from(value: &Payload) -> Result<Self, Self::Error> {
        ZSerde.deserialize(value)
    }
}

// Shared memory conversion
#[cfg(feature = "shared-memory")]
impl Serialize<Arc<SharedMemoryBuf>> for ZSerde {
    type Output = Payload;

    fn serialize(self, t: Arc<SharedMemoryBuf>) -> Self::Output {
        Payload::new(t)
    }
}

#[cfg(feature = "shared-memory")]
impl Serialize<Box<SharedMemoryBuf>> for ZSerde {
    type Output = Payload;

    fn serialize(self, t: Box<SharedMemoryBuf>) -> Self::Output {
        let smb: Arc<SharedMemoryBuf> = t.into();
        Self.serialize(smb)
    }
}

#[cfg(feature = "shared-memory")]
impl Serialize<SharedMemoryBuf> for ZSerde {
    type Output = Payload;

    fn serialize(self, t: SharedMemoryBuf) -> Self::Output {
        Payload::new(t)
    }
}

// Tuple
impl<A, B> Serialize<(A, B)> for ZSerde
where
    A: Into<Payload>,
    B: Into<Payload>,
{
    type Output = Payload;

    fn serialize(self, t: (A, B)) -> Self::Output {
        let (a, b) = t;

        let codec = Zenoh080::new();
        let mut buffer: ZBuf = ZBuf::empty();
        let mut writer = buffer.writer();
        let apld: Payload = a.into();
        let bpld: Payload = b.into();

        // SAFETY: we are serializing slices on a ZBuf, so serialization will never
        //         fail unless we run out of memory. In that case, Rust memory allocator
        //         will panic before the serializer has any chance to fail.
        unsafe {
            codec.write(&mut writer, &apld.0).unwrap_unchecked();
            codec.write(&mut writer, &bpld.0).unwrap_unchecked();
        }

        Payload::new(buffer)
    }
}

impl<'a, A, B> Deserialize<'a, (A, B)> for ZSerde
where
    A: TryFrom<Payload>,
    <A as TryFrom<Payload>>::Error: Debug,
    B: TryFrom<Payload>,
    <B as TryFrom<Payload>>::Error: Debug,
{
    type Error = ZError;

    fn deserialize(self, payload: &'a Payload) -> Result<(A, B), Self::Error> {
        let codec = Zenoh080::new();
        let mut reader = payload.0.reader();

        let abuf: ZBuf = codec.read(&mut reader).map_err(|e| zerror!("{:?}", e))?;
        let apld = Payload::new(abuf);

        let bbuf: ZBuf = codec.read(&mut reader).map_err(|e| zerror!("{:?}", e))?;
        let bpld = Payload::new(bbuf);

        let a = A::try_from(apld).map_err(|e| zerror!("{:?}", e))?;
        let b = B::try_from(bpld).map_err(|e| zerror!("{:?}", e))?;
        Ok((a, b))
    }
}

// Iterator
// impl<I, T> Serialize<I> for ZSerde
// where
//     I: Iterator<Item = T>,
//     T: Into<Payload>,
// {
//     type Output = Payload;

//     fn serialize(self, iter: I) -> Self::Output {
//         let codec = Zenoh080::new();
//         let mut buffer: ZBuf = ZBuf::empty();
//         let mut writer = buffer.writer();
//         for t in iter {
//             let tpld: Payload = t.into();
//             // SAFETY: we are serializing slices on a ZBuf, so serialization will never
//             //         fail unless we run out of memory. In that case, Rust memory allocator
//             //         will panic before the serializer has any chance to fail.
//             unsafe {
//                 codec.write(&mut writer, &tpld.0).unwrap_unchecked();
//             }
//         }

//         Payload::new(buffer)
//     }
// }

// For convenience to always convert a Value the examples
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StringOrBase64 {
    String(String),
    Base64(String),
}

impl StringOrBase64 {
    pub fn into_string(self) -> String {
        match self {
            StringOrBase64::String(s) | StringOrBase64::Base64(s) => s,
        }
    }
}

impl Deref for StringOrBase64 {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::String(s) | Self::Base64(s) => s,
        }
    }
}

impl std::fmt::Display for StringOrBase64 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self)
    }
}

impl From<&Payload> for StringOrBase64 {
    fn from(v: &Payload) -> Self {
        use base64::{engine::general_purpose::STANDARD as b64_std_engine, Engine};
        match v.deserialize::<Cow<str>>() {
            Ok(s) => StringOrBase64::String(s.into_owned()),
            Err(_) => {
                let cow: Cow<'_, [u8]> = Cow::from(v);
                StringOrBase64::Base64(b64_std_engine.encode(cow))
            }
        }
    }
}

mod tests {
    #[test]
    fn serializer() {
        use super::Payload;
        use rand::Rng;
        use zenoh_buffers::ZBuf;

        const NUM: usize = 1_000;

        macro_rules! serialize_deserialize {
            ($t:ty, $in:expr) => {
                let i = $in;
                let t = i.clone();
                println!("Serialize:\t{:?}", t);
                let v = Payload::serialize(t);
                println!("Deserialize:\t{:?}", v);
                let o: $t = v.deserialize().unwrap();
                assert_eq!(i, o);
                println!("");
            };
        }

        let mut rng = rand::thread_rng();

        // unsigned integer
        serialize_deserialize!(u8, u8::MIN);
        serialize_deserialize!(u16, u16::MIN);
        serialize_deserialize!(u32, u32::MIN);
        serialize_deserialize!(u64, u64::MIN);
        serialize_deserialize!(usize, usize::MIN);

        serialize_deserialize!(u8, u8::MAX);
        serialize_deserialize!(u16, u16::MAX);
        serialize_deserialize!(u32, u32::MAX);
        serialize_deserialize!(u64, u64::MAX);
        serialize_deserialize!(usize, usize::MAX);

        for _ in 0..NUM {
            serialize_deserialize!(u8, rng.gen::<u8>());
            serialize_deserialize!(u16, rng.gen::<u16>());
            serialize_deserialize!(u32, rng.gen::<u32>());
            serialize_deserialize!(u64, rng.gen::<u64>());
            serialize_deserialize!(usize, rng.gen::<usize>());
        }

        // signed integer
        serialize_deserialize!(i8, i8::MIN);
        serialize_deserialize!(i16, i16::MIN);
        serialize_deserialize!(i32, i32::MIN);
        serialize_deserialize!(i64, i64::MIN);
        serialize_deserialize!(isize, isize::MIN);

        serialize_deserialize!(i8, i8::MAX);
        serialize_deserialize!(i16, i16::MAX);
        serialize_deserialize!(i32, i32::MAX);
        serialize_deserialize!(i64, i64::MAX);
        serialize_deserialize!(isize, isize::MAX);

        for _ in 0..NUM {
            serialize_deserialize!(i8, rng.gen::<i8>());
            serialize_deserialize!(i16, rng.gen::<i16>());
            serialize_deserialize!(i32, rng.gen::<i32>());
            serialize_deserialize!(i64, rng.gen::<i64>());
            serialize_deserialize!(isize, rng.gen::<isize>());
        }

        // float
        serialize_deserialize!(f32, f32::MIN);
        serialize_deserialize!(f64, f64::MIN);

        serialize_deserialize!(f32, f32::MAX);
        serialize_deserialize!(f64, f64::MAX);

        for _ in 0..NUM {
            serialize_deserialize!(f32, rng.gen::<f32>());
            serialize_deserialize!(f64, rng.gen::<f64>());
        }

        // String
        serialize_deserialize!(String, "");
        serialize_deserialize!(String, String::from("abcdefghijklmnopqrstuvwxyz"));

        // Vec
        serialize_deserialize!(Vec<u8>, vec![0u8; 0]);
        serialize_deserialize!(Vec<u8>, vec![0u8; 64]);

        // ZBuf
        serialize_deserialize!(ZBuf, ZBuf::from(vec![0u8; 0]));
        serialize_deserialize!(ZBuf, ZBuf::from(vec![0u8; 64]));

        // Tuple
        serialize_deserialize!((usize, usize), (0, 1));
        serialize_deserialize!((usize, String), (0, String::from("a")));
        serialize_deserialize!((String, String), (String::from("a"), String::from("b")));

        // Iterator
        // let mut hm = Vec::new();
        // hm.push(0);
        // hm.push(1);
        // Payload::serialize(hm.iter());

        // let mut hm = HashMap::new();
        // hm.insert(0, 0);
        // hm.insert(1, 1);
        // Payload::serialize(hm.iter().map(|(k, v)| (k, v)));
        // for (k, v) in sample.payload().iter::<(String, serde_json::Value)>() {}
    }
}

// macro_rules! impl_iterator_inner {
//     ($iter:expr) => {{
//         let codec = Zenoh080::new();
//         let mut buffer: ZBuf = ZBuf::empty();
//         let mut writer = buffer.writer();
//         for t in $iter {
//             let tpld = ZSerde.serialize(t);
//             // SAFETY: we are serializing slices on a ZBuf, so serialization will never
//             //         fail unless we run out of memory. In that case, Rust memory allocator
//             //         will panic before the serializer has any chance to fail.
//             unsafe {
//                 codec.write(&mut writer, &tpld.0).unwrap_unchecked();
//             }
//         }

//         Payload::new(buffer)
//     }};
// }

// impl<'a> Serialize<std::slice::Iter<'_, i32>> for ZSerde {
//     type Output = Payload;

//     fn serialize(self, iter: std::slice::Iter<'_, i32>) -> Self::Output {
//         impl_iterator_inner!(iter)
//     }
// }

// impl<'a> Serialize<std::slice::IterMut<'_, i32>> for ZSerde {
//     type Output = Payload;

//     fn serialize(self, iter: std::slice::IterMut<'_, i32>) -> Self::Output {
//         impl_iterator_inner!(iter)
//     }
// }

// impl Serialize<&mut dyn Iterator<Item = (&i32, &i32)>> for ZSerde {
//     type Output = Payload;

//     fn serialize(self, iter: &mut dyn Iterator<Item = (&i32, &i32)>) -> Self::Output {
//         let codec = Zenoh080::new();
//         let mut buffer: ZBuf = ZBuf::empty();
//         let mut writer = buffer.writer();
//         for t in iter {
//             let tpld = ZSerde.serialize(t);
//             // SAFETY: we are serializing slices on a ZBuf, so serialization will never
//             //         fail unless we run out of memory. In that case, Rust memory allocator
//             //         will panic before the serializer has any chance to fail.
//             unsafe {
//                 codec.write(&mut writer, &tpld.0).unwrap_unchecked();
//             }
//         }

//         Payload::new(buffer)
//     }
// }

// impl<A, B> Serialize<(A, B)> for ZSerde
// where
//     ZSerde: Serialize<A, Output = Payload>,
//     ZSerde: Serialize<B, Output = Payload>,
// {
//     type Output = Payload;

//     fn serialize(self, t: (A, B)) -> Self::Output {
//         let (a, b) = t;

//         let codec = Zenoh080::new();
//         let mut buffer: ZBuf = ZBuf::empty();
//         let mut writer = buffer.writer();
//         let apld = Payload::serialize::<A>(a);
//         let bpld = Payload::serialize::<B>(b);

//         // SAFETY: we are serializing slices on a ZBuf, so serialization will never
//         //         fail unless we run out of memory. In that case, Rust memory allocator
//         //         will panic before the serializer has any chance to fail.
//         unsafe {
//             codec.write(&mut writer, &apld.0).unwrap_unchecked();
//             codec.write(&mut writer, &bpld.0).unwrap_unchecked();
//         }

//         Payload::new(buffer)
//     }
// }

// impl<'a, A, B> Deserialize<'a, (A, B)> for ZSerde
// where
//     A: TryFrom<Payload>,
//     ZSerde: Deserialize<'a, A>,
//     <ZSerde as Deserialize<'a, A>>::Error: Debug,
//     B: TryFrom<Payload>,
//     ZSerde: Deserialize<'a, B>,
//     <ZSerde as Deserialize<'a, B>>::Error: Debug,
// {
//     type Error = ZError;

//     fn deserialize(self, payload: &'a Payload) -> Result<(A, B), Self::Error> {
//         let codec = Zenoh080::new();
//         let mut reader = payload.0.reader();

//         let abuf: ZBuf = codec.read(&mut reader).map_err(|e| zerror!("{:?}", e))?;
//         let apld = Payload::new(abuf);

//         let bbuf: ZBuf = codec.read(&mut reader).map_err(|e| zerror!("{:?}", e))?;
//         let bpld = Payload::new(bbuf);

//         let a = A::try_from(apld).map_err(|e| zerror!("{:?}", e))?;
//         let b = B::try_from(bpld).map_err(|e| zerror!("{:?}", e))?;
//         Ok((a, b))
//     }
// }

// impl<T> Serialize<&mut dyn Iterator<Item = T>> for ZSerde
// where
//     ZSerde: Serialize<T, Output = Payload>,
// {
//     type Output = Payload;

//     fn serialize(self, iter: &mut dyn Iterator<Item = T>) -> Self::Output {
//         let codec = Zenoh080::new();
//         let mut buffer: ZBuf = ZBuf::empty();
//         let mut writer = buffer.writer();
//         for t in iter {
//             let tpld = ZSerde.serialize(t);
//             // SAFETY: we are serializing slices on a ZBuf, so serialization will never
//             //         fail unless we run out of memory. In that case, Rust memory allocator
//             //         will panic before the serializer has any chance to fail.
//             unsafe {
//                 codec.write(&mut writer, &tpld.0).unwrap_unchecked();
//             }
//         }

//         Payload::new(buffer)
//     }
// }

// Iterator
// macro_rules! impl_iterator_serialize {
//     ($a:ty) => {
//         impl Serialize<&mut dyn Iterator<Item = $a>> for ZSerde
//         {
//             type Output = Payload;

//             fn serialize(self, iter: &mut dyn Iterator<Item = $a>) -> Self::Output {
//                 let codec = Zenoh080::new();
//                 let mut buffer: ZBuf = ZBuf::empty();
//                 let mut writer = buffer.writer();
//                 for t in iter {
//                     let tpld = ZSerde.serialize(t);
//                     // SAFETY: we are serializing slices on a ZBuf, so serialization will never
//                     //         fail unless we run out of memory. In that case, Rust memory allocator
//                     //         will panic before the serializer has any chance to fail.
//                     unsafe {
//                         codec.write(&mut writer, &tpld.0).unwrap_unchecked();
//                     }
//                 }

//                 Payload::new(buffer)
//             }
//         }
//     };
// }

// Tuples
// macro_rules! impl_tuple_serialize {
//     ($a:ty, $b:ty) => {
//         impl Serialize<($a, $b)> for ZSerde
//         {
//             type Output = Payload;

//             fn serialize(self, t: ($a, $b)) -> Self::Output {
//                 let (a, b) = t;

//                 let codec = Zenoh080::new();
//                 let mut buffer: ZBuf = ZBuf::empty();
//                 let mut writer = buffer.writer();
//                 let apld = Payload::serialize::<$a>(a);
//                 let bpld = Payload::serialize::<$b>(b);

//                 // SAFETY: we are serializing slices on a ZBuf, so serialization will never
//                 //         fail unless we run out of memory. In that case, Rust memory allocator
//                 //         will panic before the serializer has any chance to fail.
//                 unsafe {
//                     codec.write(&mut writer, &apld.0).unwrap_unchecked();
//                     codec.write(&mut writer, &bpld.0).unwrap_unchecked();
//                 }

//                 Payload::new(buffer)
//             }
//         }
//     }

// }

// macro_rules! impl_tuple_deserialize {
//     ($a:ty, $b:ty) => {
//         impl<'a> Deserialize<'a, ($a, $b)> for ZSerde {
//             type Error = ZError;

//             fn deserialize(self, payload: &'a Payload) -> Result<($a, $b), Self::Error> {
//                 let codec = Zenoh080::new();
//                 let mut reader = payload.0.reader();

//                 let abuf: ZBuf = codec.read(&mut reader).map_err(|e| zerror!("{:?}", e))?;
//                 let apld = Payload::new(abuf);

//                 let bbuf: ZBuf = codec.read(&mut reader).map_err(|e| zerror!("{:?}", e))?;
//                 let bpld = Payload::new(bbuf);

//                 let a = apld.deserialize::<$a>().map_err(|e| zerror!("{:?}", e))?;
//                 let b = bpld.deserialize::<$b>().map_err(|e| zerror!("{:?}", e))?;
//                 Ok((a, b))
//             }
//         }
//     };
// }

// impl_tuple_serialize!(u8, u8);
// impl_tuple_deserialize!(u8, u8);
// impl_tuple_serialize!(u8, u16);
// impl_tuple_deserialize!(u8, u16);
// impl_tuple_serialize!(u8, u32);
// impl_tuple_deserialize!(u8, u32);
// impl_tuple_serialize!(u8, u64);
// impl_tuple_deserialize!(u8, u64);
// impl_tuple_serialize!(u8, usize);
// impl_tuple_deserialize!(u8, usize);
// impl_tuple_serialize!(u8, i8);
// impl_tuple_deserialize!(u8, i8);
// impl_tuple_serialize!(u8, i16);
// impl_tuple_deserialize!(u8, i16);
// impl_tuple_serialize!(u8, i32);
// impl_tuple_deserialize!(u8, i32);
// impl_tuple_serialize!(u8, isize);
// impl_tuple_deserialize!(u8, isize);
// impl_tuple_serialize!(u8, f32);
// impl_tuple_deserialize!(u8, f32);
// impl_tuple_serialize!(u8, f64);
// impl_tuple_deserialize!(u8, f64);
// impl_tuple_serialize!(u8, bool);
// impl_tuple_deserialize!(u8, bool);
// impl_tuple_serialize!(u8, ZBuf);
// impl_tuple_deserialize!(u8, ZBuf);
// impl_tuple_serialize!(u8, Vec<u8>);
// impl_tuple_deserialize!(u8, Vec<u8>);
// impl_tuple_serialize!(u8, String);
// impl_tuple_deserialize!(u8, String);
// impl_tuple_serialize!(u8, &[u8]);
// impl_tuple_serialize!(u16, u8);
// impl_tuple_deserialize!(u16, u8);
// impl_tuple_serialize!(u16, u16);
// impl_tuple_deserialize!(u16, u16);
// impl_tuple_serialize!(u16, u32);
// impl_tuple_deserialize!(u16, u32);
// impl_tuple_serialize!(u16, u64);
// impl_tuple_deserialize!(u16, u64);
// impl_tuple_serialize!(u16, usize);
// impl_tuple_deserialize!(u16, usize);
// impl_tuple_serialize!(u16, i8);
// impl_tuple_deserialize!(u16, i8);
// impl_tuple_serialize!(u16, i16);
// impl_tuple_deserialize!(u16, i16);
// impl_tuple_serialize!(u16, i32);
// impl_tuple_deserialize!(u16, i32);
// impl_tuple_serialize!(u16, isize);
// impl_tuple_deserialize!(u16, isize);
// impl_tuple_serialize!(u16, f32);
// impl_tuple_deserialize!(u16, f32);
// impl_tuple_serialize!(u16, f64);
// impl_tuple_deserialize!(u16, f64);
// impl_tuple_serialize!(u16, bool);
// impl_tuple_deserialize!(u16, bool);
// impl_tuple_serialize!(u16, ZBuf);
// impl_tuple_deserialize!(u16, ZBuf);
// impl_tuple_serialize!(u16, Vec<u8>);
// impl_tuple_deserialize!(u16, Vec<u8>);
// impl_tuple_serialize!(u16, String);
// impl_tuple_deserialize!(u16, String);
// impl_tuple_serialize!(u16, &[u8]);
// impl_tuple_serialize!(u32, u8);
// impl_tuple_deserialize!(u32, u8);
// impl_tuple_serialize!(u32, u16);
// impl_tuple_deserialize!(u32, u16);
// impl_tuple_serialize!(u32, u32);
// impl_tuple_deserialize!(u32, u32);
// impl_tuple_serialize!(u32, u64);
// impl_tuple_deserialize!(u32, u64);
// impl_tuple_serialize!(u32, usize);
// impl_tuple_deserialize!(u32, usize);
// impl_tuple_serialize!(u32, i8);
// impl_tuple_deserialize!(u32, i8);
// impl_tuple_serialize!(u32, i16);
// impl_tuple_deserialize!(u32, i16);
// impl_tuple_serialize!(u32, i32);
// impl_tuple_deserialize!(u32, i32);
// impl_tuple_serialize!(u32, isize);
// impl_tuple_deserialize!(u32, isize);
// impl_tuple_serialize!(u32, f32);
// impl_tuple_deserialize!(u32, f32);
// impl_tuple_serialize!(u32, f64);
// impl_tuple_deserialize!(u32, f64);
// impl_tuple_serialize!(u32, bool);
// impl_tuple_deserialize!(u32, bool);
// impl_tuple_serialize!(u32, ZBuf);
// impl_tuple_deserialize!(u32, ZBuf);
// impl_tuple_serialize!(u32, Vec<u8>);
// impl_tuple_deserialize!(u32, Vec<u8>);
// impl_tuple_serialize!(u32, String);
// impl_tuple_deserialize!(u32, String);
// impl_tuple_serialize!(u32, &[u8]);
// impl_tuple_serialize!(u64, u8);
// impl_tuple_deserialize!(u64, u8);
// impl_tuple_serialize!(u64, u16);
// impl_tuple_deserialize!(u64, u16);
// impl_tuple_serialize!(u64, u32);
// impl_tuple_deserialize!(u64, u32);
// impl_tuple_serialize!(u64, u64);
// impl_tuple_deserialize!(u64, u64);
// impl_tuple_serialize!(u64, usize);
// impl_tuple_deserialize!(u64, usize);
// impl_tuple_serialize!(u64, i8);
// impl_tuple_deserialize!(u64, i8);
// impl_tuple_serialize!(u64, i16);
// impl_tuple_deserialize!(u64, i16);
// impl_tuple_serialize!(u64, i32);
// impl_tuple_deserialize!(u64, i32);
// impl_tuple_serialize!(u64, isize);
// impl_tuple_deserialize!(u64, isize);
// impl_tuple_serialize!(u64, f32);
// impl_tuple_deserialize!(u64, f32);
// impl_tuple_serialize!(u64, f64);
// impl_tuple_deserialize!(u64, f64);
// impl_tuple_serialize!(u64, bool);
// impl_tuple_deserialize!(u64, bool);
// impl_tuple_serialize!(u64, ZBuf);
// impl_tuple_deserialize!(u64, ZBuf);
// impl_tuple_serialize!(u64, Vec<u8>);
// impl_tuple_deserialize!(u64, Vec<u8>);
// impl_tuple_serialize!(u64, String);
// impl_tuple_deserialize!(u64, String);
// impl_tuple_serialize!(u64, &[u8]);
// impl_tuple_serialize!(usize, u8);
// impl_tuple_deserialize!(usize, u8);
// impl_tuple_serialize!(usize, u16);
// impl_tuple_deserialize!(usize, u16);
// impl_tuple_serialize!(usize, u32);
// impl_tuple_deserialize!(usize, u32);
// impl_tuple_serialize!(usize, u64);
// impl_tuple_deserialize!(usize, u64);
// impl_tuple_serialize!(usize, usize);
// impl_tuple_deserialize!(usize, usize);
// impl_tuple_serialize!(usize, i8);
// impl_tuple_deserialize!(usize, i8);
// impl_tuple_serialize!(usize, i16);
// impl_tuple_deserialize!(usize, i16);
// impl_tuple_serialize!(usize, i32);
// impl_tuple_deserialize!(usize, i32);
// impl_tuple_serialize!(usize, isize);
// impl_tuple_deserialize!(usize, isize);
// impl_tuple_serialize!(usize, f32);
// impl_tuple_deserialize!(usize, f32);
// impl_tuple_serialize!(usize, f64);
// impl_tuple_deserialize!(usize, f64);
// impl_tuple_serialize!(usize, bool);
// impl_tuple_deserialize!(usize, bool);
// impl_tuple_serialize!(usize, ZBuf);
// impl_tuple_deserialize!(usize, ZBuf);
// impl_tuple_serialize!(usize, Vec<u8>);
// impl_tuple_deserialize!(usize, Vec<u8>);
// impl_tuple_serialize!(usize, String);
// impl_tuple_deserialize!(usize, String);
// impl_tuple_serialize!(usize, &[u8]);
// impl_tuple_serialize!(i8, u8);
// impl_tuple_deserialize!(i8, u8);
// impl_tuple_serialize!(i8, u16);
// impl_tuple_deserialize!(i8, u16);
// impl_tuple_serialize!(i8, u32);
// impl_tuple_deserialize!(i8, u32);
// impl_tuple_serialize!(i8, u64);
// impl_tuple_deserialize!(i8, u64);
// impl_tuple_serialize!(i8, usize);
// impl_tuple_deserialize!(i8, usize);
// impl_tuple_serialize!(i8, i8);
// impl_tuple_deserialize!(i8, i8);
// impl_tuple_serialize!(i8, i16);
// impl_tuple_deserialize!(i8, i16);
// impl_tuple_serialize!(i8, i32);
// impl_tuple_deserialize!(i8, i32);
// impl_tuple_serialize!(i8, isize);
// impl_tuple_deserialize!(i8, isize);
// impl_tuple_serialize!(i8, f32);
// impl_tuple_deserialize!(i8, f32);
// impl_tuple_serialize!(i8, f64);
// impl_tuple_deserialize!(i8, f64);
// impl_tuple_serialize!(i8, bool);
// impl_tuple_deserialize!(i8, bool);
// impl_tuple_serialize!(i8, ZBuf);
// impl_tuple_deserialize!(i8, ZBuf);
// impl_tuple_serialize!(i8, Vec<u8>);
// impl_tuple_deserialize!(i8, Vec<u8>);
// impl_tuple_serialize!(i8, String);
// impl_tuple_deserialize!(i8, String);
// impl_tuple_serialize!(i8, &[u8]);
// impl_tuple_serialize!(i16, u8);
// impl_tuple_deserialize!(i16, u8);
// impl_tuple_serialize!(i16, u16);
// impl_tuple_deserialize!(i16, u16);
// impl_tuple_serialize!(i16, u32);
// impl_tuple_deserialize!(i16, u32);
// impl_tuple_serialize!(i16, u64);
// impl_tuple_deserialize!(i16, u64);
// impl_tuple_serialize!(i16, usize);
// impl_tuple_deserialize!(i16, usize);
// impl_tuple_serialize!(i16, i8);
// impl_tuple_deserialize!(i16, i8);
// impl_tuple_serialize!(i16, i16);
// impl_tuple_deserialize!(i16, i16);
// impl_tuple_serialize!(i16, i32);
// impl_tuple_deserialize!(i16, i32);
// impl_tuple_serialize!(i16, isize);
// impl_tuple_deserialize!(i16, isize);
// impl_tuple_serialize!(i16, f32);
// impl_tuple_deserialize!(i16, f32);
// impl_tuple_serialize!(i16, f64);
// impl_tuple_deserialize!(i16, f64);
// impl_tuple_serialize!(i16, bool);
// impl_tuple_deserialize!(i16, bool);
// impl_tuple_serialize!(i16, ZBuf);
// impl_tuple_deserialize!(i16, ZBuf);
// impl_tuple_serialize!(i16, Vec<u8>);
// impl_tuple_deserialize!(i16, Vec<u8>);
// impl_tuple_serialize!(i16, String);
// impl_tuple_deserialize!(i16, String);
// impl_tuple_serialize!(i16, &[u8]);
// impl_tuple_serialize!(i32, u8);
// impl_tuple_deserialize!(i32, u8);
// impl_tuple_serialize!(i32, u16);
// impl_tuple_deserialize!(i32, u16);
// impl_tuple_serialize!(i32, u32);
// impl_tuple_deserialize!(i32, u32);
// impl_tuple_serialize!(i32, u64);
// impl_tuple_deserialize!(i32, u64);
// impl_tuple_serialize!(i32, usize);
// impl_tuple_deserialize!(i32, usize);
// impl_tuple_serialize!(i32, i8);
// impl_tuple_deserialize!(i32, i8);
// impl_tuple_serialize!(i32, i16);
// impl_tuple_deserialize!(i32, i16);
// impl_tuple_serialize!(i32, i32);
// impl_tuple_deserialize!(i32, i32);
// impl_tuple_serialize!(i32, isize);
// impl_tuple_deserialize!(i32, isize);
// impl_tuple_serialize!(i32, f32);
// impl_tuple_deserialize!(i32, f32);
// impl_tuple_serialize!(i32, f64);
// impl_tuple_deserialize!(i32, f64);
// impl_tuple_serialize!(i32, bool);
// impl_tuple_deserialize!(i32, bool);
// impl_tuple_serialize!(i32, ZBuf);
// impl_tuple_deserialize!(i32, ZBuf);
// impl_tuple_serialize!(i32, Vec<u8>);
// impl_tuple_deserialize!(i32, Vec<u8>);
// impl_tuple_serialize!(i32, String);
// impl_tuple_deserialize!(i32, String);
// impl_tuple_serialize!(i32, &[u8]);
// impl_tuple_serialize!(isize, u8);
// impl_tuple_deserialize!(isize, u8);
// impl_tuple_serialize!(isize, u16);
// impl_tuple_deserialize!(isize, u16);
// impl_tuple_serialize!(isize, u32);
// impl_tuple_deserialize!(isize, u32);
// impl_tuple_serialize!(isize, u64);
// impl_tuple_deserialize!(isize, u64);
// impl_tuple_serialize!(isize, usize);
// impl_tuple_deserialize!(isize, usize);
// impl_tuple_serialize!(isize, i8);
// impl_tuple_deserialize!(isize, i8);
// impl_tuple_serialize!(isize, i16);
// impl_tuple_deserialize!(isize, i16);
// impl_tuple_serialize!(isize, i32);
// impl_tuple_deserialize!(isize, i32);
// impl_tuple_serialize!(isize, isize);
// impl_tuple_deserialize!(isize, isize);
// impl_tuple_serialize!(isize, f32);
// impl_tuple_deserialize!(isize, f32);
// impl_tuple_serialize!(isize, f64);
// impl_tuple_deserialize!(isize, f64);
// impl_tuple_serialize!(isize, bool);
// impl_tuple_deserialize!(isize, bool);
// impl_tuple_serialize!(isize, ZBuf);
// impl_tuple_deserialize!(isize, ZBuf);
// impl_tuple_serialize!(isize, Vec<u8>);
// impl_tuple_deserialize!(isize, Vec<u8>);
// impl_tuple_serialize!(isize, String);
// impl_tuple_deserialize!(isize, String);
// impl_tuple_serialize!(isize, &[u8]);
// impl_tuple_serialize!(f32, u8);
// impl_tuple_deserialize!(f32, u8);
// impl_tuple_serialize!(f32, u16);
// impl_tuple_deserialize!(f32, u16);
// impl_tuple_serialize!(f32, u32);
// impl_tuple_deserialize!(f32, u32);
// impl_tuple_serialize!(f32, u64);
// impl_tuple_deserialize!(f32, u64);
// impl_tuple_serialize!(f32, usize);
// impl_tuple_deserialize!(f32, usize);
// impl_tuple_serialize!(f32, i8);
// impl_tuple_deserialize!(f32, i8);
// impl_tuple_serialize!(f32, i16);
// impl_tuple_deserialize!(f32, i16);
// impl_tuple_serialize!(f32, i32);
// impl_tuple_deserialize!(f32, i32);
// impl_tuple_serialize!(f32, isize);
// impl_tuple_deserialize!(f32, isize);
// impl_tuple_serialize!(f32, f32);
// impl_tuple_deserialize!(f32, f32);
// impl_tuple_serialize!(f32, f64);
// impl_tuple_deserialize!(f32, f64);
// impl_tuple_serialize!(f32, bool);
// impl_tuple_deserialize!(f32, bool);
// impl_tuple_serialize!(f32, ZBuf);
// impl_tuple_deserialize!(f32, ZBuf);
// impl_tuple_serialize!(f32, Vec<u8>);
// impl_tuple_deserialize!(f32, Vec<u8>);
// impl_tuple_serialize!(f32, String);
// impl_tuple_deserialize!(f32, String);
// impl_tuple_serialize!(f32, &[u8]);
// impl_tuple_serialize!(f64, u8);
// impl_tuple_deserialize!(f64, u8);
// impl_tuple_serialize!(f64, u16);
// impl_tuple_deserialize!(f64, u16);
// impl_tuple_serialize!(f64, u32);
// impl_tuple_deserialize!(f64, u32);
// impl_tuple_serialize!(f64, u64);
// impl_tuple_deserialize!(f64, u64);
// impl_tuple_serialize!(f64, usize);
// impl_tuple_deserialize!(f64, usize);
// impl_tuple_serialize!(f64, i8);
// impl_tuple_deserialize!(f64, i8);
// impl_tuple_serialize!(f64, i16);
// impl_tuple_deserialize!(f64, i16);
// impl_tuple_serialize!(f64, i32);
// impl_tuple_deserialize!(f64, i32);
// impl_tuple_serialize!(f64, isize);
// impl_tuple_deserialize!(f64, isize);
// impl_tuple_serialize!(f64, f32);
// impl_tuple_deserialize!(f64, f32);
// impl_tuple_serialize!(f64, f64);
// impl_tuple_deserialize!(f64, f64);
// impl_tuple_serialize!(f64, bool);
// impl_tuple_deserialize!(f64, bool);
// impl_tuple_serialize!(f64, ZBuf);
// impl_tuple_deserialize!(f64, ZBuf);
// impl_tuple_serialize!(f64, Vec<u8>);
// impl_tuple_deserialize!(f64, Vec<u8>);
// impl_tuple_serialize!(f64, String);
// impl_tuple_deserialize!(f64, String);
// impl_tuple_serialize!(f64, &[u8]);
// impl_tuple_serialize!(bool, u8);
// impl_tuple_deserialize!(bool, u8);
// impl_tuple_serialize!(bool, u16);
// impl_tuple_deserialize!(bool, u16);
// impl_tuple_serialize!(bool, u32);
// impl_tuple_deserialize!(bool, u32);
// impl_tuple_serialize!(bool, u64);
// impl_tuple_deserialize!(bool, u64);
// impl_tuple_serialize!(bool, usize);
// impl_tuple_deserialize!(bool, usize);
// impl_tuple_serialize!(bool, i8);
// impl_tuple_deserialize!(bool, i8);
// impl_tuple_serialize!(bool, i16);
// impl_tuple_deserialize!(bool, i16);
// impl_tuple_serialize!(bool, i32);
// impl_tuple_deserialize!(bool, i32);
// impl_tuple_serialize!(bool, isize);
// impl_tuple_deserialize!(bool, isize);
// impl_tuple_serialize!(bool, f32);
// impl_tuple_deserialize!(bool, f32);
// impl_tuple_serialize!(bool, f64);
// impl_tuple_deserialize!(bool, f64);
// impl_tuple_serialize!(bool, bool);
// impl_tuple_deserialize!(bool, bool);
// impl_tuple_serialize!(bool, ZBuf);
// impl_tuple_deserialize!(bool, ZBuf);
// impl_tuple_serialize!(bool, Vec<u8>);
// impl_tuple_deserialize!(bool, Vec<u8>);
// impl_tuple_serialize!(bool, String);
// impl_tuple_deserialize!(bool, String);
// impl_tuple_serialize!(bool, &[u8]);
// impl_tuple_serialize!(ZBuf, u8);
// impl_tuple_deserialize!(ZBuf, u8);
// impl_tuple_serialize!(ZBuf, u16);
// impl_tuple_deserialize!(ZBuf, u16);
// impl_tuple_serialize!(ZBuf, u32);
// impl_tuple_deserialize!(ZBuf, u32);
// impl_tuple_serialize!(ZBuf, u64);
// impl_tuple_deserialize!(ZBuf, u64);
// impl_tuple_serialize!(ZBuf, usize);
// impl_tuple_deserialize!(ZBuf, usize);
// impl_tuple_serialize!(ZBuf, i8);
// impl_tuple_deserialize!(ZBuf, i8);
// impl_tuple_serialize!(ZBuf, i16);
// impl_tuple_deserialize!(ZBuf, i16);
// impl_tuple_serialize!(ZBuf, i32);
// impl_tuple_deserialize!(ZBuf, i32);
// impl_tuple_serialize!(ZBuf, isize);
// impl_tuple_deserialize!(ZBuf, isize);
// impl_tuple_serialize!(ZBuf, f32);
// impl_tuple_deserialize!(ZBuf, f32);
// impl_tuple_serialize!(ZBuf, f64);
// impl_tuple_deserialize!(ZBuf, f64);
// impl_tuple_serialize!(ZBuf, bool);
// impl_tuple_deserialize!(ZBuf, bool);
// impl_tuple_serialize!(ZBuf, ZBuf);
// impl_tuple_deserialize!(ZBuf, ZBuf);
// impl_tuple_serialize!(ZBuf, Vec<u8>);
// impl_tuple_deserialize!(ZBuf, Vec<u8>);
// impl_tuple_serialize!(ZBuf, String);
// impl_tuple_deserialize!(ZBuf, String);
// impl_tuple_serialize!(ZBuf, &[u8]);
// impl_tuple_serialize!(Vec<u8>, u8);
// impl_tuple_deserialize!(Vec<u8>, u8);
// impl_tuple_serialize!(Vec<u8>, u16);
// impl_tuple_deserialize!(Vec<u8>, u16);
// impl_tuple_serialize!(Vec<u8>, u32);
// impl_tuple_deserialize!(Vec<u8>, u32);
// impl_tuple_serialize!(Vec<u8>, u64);
// impl_tuple_deserialize!(Vec<u8>, u64);
// impl_tuple_serialize!(Vec<u8>, usize);
// impl_tuple_deserialize!(Vec<u8>, usize);
// impl_tuple_serialize!(Vec<u8>, i8);
// impl_tuple_deserialize!(Vec<u8>, i8);
// impl_tuple_serialize!(Vec<u8>, i16);
// impl_tuple_deserialize!(Vec<u8>, i16);
// impl_tuple_serialize!(Vec<u8>, i32);
// impl_tuple_deserialize!(Vec<u8>, i32);
// impl_tuple_serialize!(Vec<u8>, isize);
// impl_tuple_deserialize!(Vec<u8>, isize);
// impl_tuple_serialize!(Vec<u8>, f32);
// impl_tuple_deserialize!(Vec<u8>, f32);
// impl_tuple_serialize!(Vec<u8>, f64);
// impl_tuple_deserialize!(Vec<u8>, f64);
// impl_tuple_serialize!(Vec<u8>, bool);
// impl_tuple_deserialize!(Vec<u8>, bool);
// impl_tuple_serialize!(Vec<u8>, ZBuf);
// impl_tuple_deserialize!(Vec<u8>, ZBuf);
// impl_tuple_serialize!(Vec<u8>, Vec<u8>);
// impl_tuple_deserialize!(Vec<u8>, Vec<u8>);
// impl_tuple_serialize!(Vec<u8>, String);
// impl_tuple_deserialize!(Vec<u8>, String);
// impl_tuple_serialize!(Vec<u8>, &[u8]);
// impl_tuple_serialize!(String, u8);
// impl_tuple_deserialize!(String, u8);
// impl_tuple_serialize!(String, u16);
// impl_tuple_deserialize!(String, u16);
// impl_tuple_serialize!(String, u32);
// impl_tuple_deserialize!(String, u32);
// impl_tuple_serialize!(String, u64);
// impl_tuple_deserialize!(String, u64);
// impl_tuple_serialize!(String, usize);
// impl_tuple_deserialize!(String, usize);
// impl_tuple_serialize!(String, i8);
// impl_tuple_deserialize!(String, i8);
// impl_tuple_serialize!(String, i16);
// impl_tuple_deserialize!(String, i16);
// impl_tuple_serialize!(String, i32);
// impl_tuple_deserialize!(String, i32);
// impl_tuple_serialize!(String, isize);
// impl_tuple_deserialize!(String, isize);
// impl_tuple_serialize!(String, f32);
// impl_tuple_deserialize!(String, f32);
// impl_tuple_serialize!(String, f64);
// impl_tuple_deserialize!(String, f64);
// impl_tuple_serialize!(String, bool);
// impl_tuple_deserialize!(String, bool);
// impl_tuple_serialize!(String, ZBuf);
// impl_tuple_deserialize!(String, ZBuf);
// impl_tuple_serialize!(String, Vec<u8>);
// impl_tuple_deserialize!(String, Vec<u8>);
// impl_tuple_serialize!(String, String);
// impl_tuple_deserialize!(String, String);
// impl_tuple_serialize!(String, &[u8]);
// impl_tuple_serialize!(&[u8], u8);
// impl_tuple_serialize!(&[u8], u16);
// impl_tuple_serialize!(&[u8], u32);
// impl_tuple_serialize!(&[u8], u64);
// impl_tuple_serialize!(&[u8], usize);
// impl_tuple_serialize!(&[u8], i8);
// impl_tuple_serialize!(&[u8], i16);
// impl_tuple_serialize!(&[u8], i32);
// impl_tuple_serialize!(&[u8], isize);
// impl_tuple_serialize!(&[u8], f32);
// impl_tuple_serialize!(&[u8], f64);
// impl_tuple_serialize!(&[u8], bool);
// impl_tuple_serialize!(&[u8], ZBuf);
// impl_tuple_serialize!(&[u8], Vec<u8>);
// impl_tuple_serialize!(&[u8], String);
// impl_tuple_serialize!(&[u8], &[u8]);
// impl_iterator_serialize!(u8);
// impl_iterator_serialize!(u16);
// impl_iterator_serialize!(u32);
// impl_iterator_serialize!(u64);
// impl_iterator_serialize!(usize);
// impl_iterator_serialize!(i8);
// impl_iterator_serialize!(i16);
// impl_iterator_serialize!(i32);
// impl_iterator_serialize!(isize);
// impl_iterator_serialize!(f32);
// impl_iterator_serialize!(f64);
// impl_iterator_serialize!(bool);
// impl_iterator_serialize!(ZBuf);
// impl_iterator_serialize!(Vec<u8>);
// impl_iterator_serialize!(String);
// impl_iterator_serialize!(&[u8]);

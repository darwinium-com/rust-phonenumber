// Copyright (C) 2017 1aim GmbH
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::sync::Arc;
use std::path::Path;
use std::fs::File;
use std::io::{Cursor, BufReader};
use std::borrow::Borrow;
use std::hash::Hash;

use fnv::FnvHashMap;
use regex_cache::{LazyRegexBuilder, LazyRegex};
use bincode;

use error::{self, Result, Error};
use metadata::loader;

/// The Google provided metadata database, used as default.
const DATABASE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/database.bin"));

lazy_static! {
	pub static ref DEFAULT: Database =
		Database::from(bincode::deserialize(DATABASE).unwrap()).unwrap();
}

/// Representation of a database of metadata for phone number.
#[derive(Clone, Debug)]
pub struct Database {
	by_id:   FnvHashMap<String, Arc<super::Metadata>>,
	by_code: FnvHashMap<u16, Vec<Arc<super::Metadata>>>,
	regions: FnvHashMap<u16, Vec<String>>,
}

impl Database {
	/// Load a database from the given file.
	pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
		Database::from(loader::load(BufReader::new(File::open(path)?))?)
	}

	/// Parse a database from the given string.
	pub fn parse<S: AsRef<str>>(content: S) -> Result<Self> {
		Database::from(loader::load(Cursor::new(content.as_ref()))?)
	}

	/// Create a database from a loaded database.
	pub fn from(meta: Vec<loader::Metadata>) -> Result<Self> {
		fn switch<T>(value: Option<Result<T>>) -> Result<Option<T>> {
			match value {
				None =>
					Ok(None),

				Some(Ok(value)) =>
					Ok(Some(value)),

				Some(Err(err)) =>
					Err(err),
			}
		}

		fn regex(value: String) -> Result<LazyRegex> {
			Ok(LazyRegexBuilder::new(&value).ignore_whitespace(true).build()?)
		}

		fn metadata(meta: loader::Metadata) -> Result<super::Metadata> {
			Ok(super::Metadata {
				descriptors: super::Descriptors {
					general: descriptor(meta.general.ok_or_else(||
						Error::from(error::Metadata::MissingValue {
							phase: "metadata".into(),
							name:  "generalDesc".into(),
						}))?)?,

					fixed_line:       switch(meta.fixed_line.map(descriptor))?,
					mobile:           switch(meta.mobile.map(descriptor))?,
					toll_free:        switch(meta.toll_free.map(descriptor))?,
					premium_rate:     switch(meta.premium_rate.map(descriptor))?,
					shared_cost:      switch(meta.shared_cost.map(descriptor))?,
					personal_number:  switch(meta.personal_number.map(descriptor))?,
					voip:             switch(meta.voip.map(descriptor))?,
					pager:            switch(meta.pager.map(descriptor))?,
					uan:              switch(meta.uan.map(descriptor))?,
					emergency:        switch(meta.emergency.map(descriptor))?,
					voicemail:        switch(meta.voicemail.map(descriptor))?,
					short_code:       switch(meta.short_code.map(descriptor))?,
					standard_rate:    switch(meta.standard_rate.map(descriptor))?,
					carrier:          switch(meta.carrier.map(descriptor))?,
					no_international: switch(meta.no_international.map(descriptor))?,
				},

				id: meta.id.ok_or_else(||
					Error::from(error::Metadata::MissingValue {
						phase: "metadata".into(),
						name:  "id".into()
					}))?,

				country_code: meta.country_code.ok_or_else(||
					Error::from(error::Metadata::MissingValue {
						phase: "metadata".into(),
						name: "countryCode".into(),
					}))?,

				international_prefix: switch(meta.international_prefix.map(regex))?,
				preferred_international_prefix: meta.preferred_international_prefix,
				national_prefix: meta.national_prefix,
				preferred_extension_prefix: meta.preferred_extension_prefix,
				national_prefix_for_parsing: switch(meta.national_prefix_for_parsing.map(regex))?,
				national_prefix_transform_rule: meta.national_prefix_transform_rule,

				format: meta.format.into_iter().map(format).collect::<Result<_>>()?,
				international_format: meta.international_format.into_iter().map(format).collect::<Result<_>>()?,

				main_country_for_code: meta.main_country_for_code,
				leading_digits: switch(meta.leading_digits.map(regex))?,
				mobile_number_portable: meta.mobile_number_portable,
			})
		}

		fn descriptor(desc: loader::Descriptor) -> Result<super::Descriptor> {
			desc.national_number.as_ref().unwrap();
			desc.national_number.as_ref().unwrap();

			Ok(super::Descriptor {
				national_number: desc.national_number.ok_or_else(||
					Error::from(error::Metadata::MissingValue {
						phase: "descriptor".into(),
						name:  "national_number".into(),
					})).and_then(regex)?,

				possible_number: switch(desc.possible_number.map(regex))?,
				possible_length: desc.possible_length,
				possible_local_length: desc.possible_local_length,
				example: desc.example,
			})
		}

		fn format(format: loader::Format) -> Result<super::Format> {
			Ok(super::Format {
				pattern: format.pattern.ok_or_else(||
					Error::from(error::Metadata::MissingValue {
						phase: "format".into(),
						name:  "pattern".into(),
					})).and_then(regex)?,

				format: format.format.ok_or_else(||
					Error::from(error::Metadata::MissingValue {
						phase: "format".into(),
						name:  "format".into()
					}))?,

				leading_digits: format.leading_digits.into_iter()
					.map(regex).collect::<Result<_>>()?,

				national_prefix: format.national_prefix,
				domestic_carrier: format.domestic_carrier,
			})
		}

		let mut by_id   = FnvHashMap::default();
		let mut by_code = FnvHashMap::default();
		let mut regions = FnvHashMap::default();

		for meta in meta {
			let meta = Arc::new(metadata(meta)?);

			by_id.insert(meta.id.clone(), meta.clone());
			by_code.entry(meta.country_code).or_insert_with(Vec::new).push(meta.clone());
			regions.entry(meta.country_code).or_insert_with(Vec::new).push(meta.id.clone());
		}

		Ok(Database {
			by_id:   by_id,
			by_code: by_code,
			regions: regions,
		})
	}

	/// Get a metadata entry by country ID.
	pub fn by_id<Q>(&self, key: &Q) -> Option<&super::Metadata>
		where Q:      ?Sized + Hash + Eq,
		      String: Borrow<Q>,
	{
		self.by_id.get(key).map(AsRef::as_ref)
	}

	/// Get metadata entries by country code.
	pub fn by_code<Q>(&self, key: &Q) -> Option<Vec<&super::Metadata>>
		where Q:   ?Sized + Hash + Eq,
		      u16: Borrow<Q>,
	{
		self.by_code.get(key).map(|m| m.iter().map(AsRef::as_ref).collect())
	}

	/// Get all country IDs corresponding to the given country code.
	pub fn region<Q>(&self, code: &Q) -> Option<Vec<&str>>
		where Q:   ?Sized + Hash + Eq,
		      u16: Borrow<Q>
	{
		self.regions.get(code).map(|m| m.iter().map(AsRef::as_ref).collect())
	}
}

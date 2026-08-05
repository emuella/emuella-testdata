# Transformation recipes

Recipes define how a selected or derived pack is produced without overwriting
the source pack. A locked derived pack records exact input digests, tool and
recipe versions, parameters, output digests, color semantics, and whether the
operation is considered an adaptation under the source terms.

Recipes must not rely on implicit color profiles, locale, current time,
unseeded randomness, host endianness, or floating dependencies. A change to a
recipe or tool that changes bytes creates a new derived pack version.

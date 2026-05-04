pub trait ToC<Raw> {
    type Owned;

    fn to_c(&self, owned: &mut Self::Owned) -> Raw;
}

pub trait FromC<Raw> {
    fn from_c(raw: Raw) -> Self;
}

pub trait FromCWith<Raw, Owned> {
    fn from_c(raw: Raw, owned: Owned) -> Self;
}

pub trait ToBytes {
    fn to_bytes(&self) -> Vec<u8>;
}

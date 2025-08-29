use std::{ptr, slice};

/// # Safety
///
/// same as `slice::from_raw_parts` but data can be null if len is zero
pub unsafe fn call_param_to_slice<'a, T>(data: *const T, len: usize) -> &'a [T] {
    if len == 0 {
        &[]
    } else {
        assert!(!data.is_null());
        slice::from_raw_parts::<'a, T>(data, len)
    }
}

/// # Safety
///
/// same as `slice::from_raw_parts_mut` but data can be null if len is zero
pub unsafe fn call_param_to_slice_mut<'a, T>(data: *mut T, len: usize) -> &'a mut [T] {
    if len == 0 {
        &mut []
    } else {
        assert!(!data.is_null());
        slice::from_raw_parts_mut::<'a, T>(data, len)
    }
}

pub fn call_param_to_ptr<T>(slice: &[T]) -> (*const T, usize) {
    if slice.is_empty() {
        (ptr::null(), 0)
    } else {
        (slice.as_ptr(), slice.len())
    }
}

pub fn call_param_to_ptr_mut<T>(slice: &mut [T]) -> (*mut T, usize) {
    if slice.is_empty() {
        (ptr::null_mut(), 0)
    } else {
        (slice.as_mut_ptr(), slice.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_param_to_ptr_empty() {
        let (ptr, len) = call_param_to_ptr(b"");
        assert!(ptr.is_null());
        assert_eq!(len, 0);
    }

    #[test]
    fn call_param_to_ptr_one() {
        let (ptr, len) = call_param_to_ptr(b"1");
        assert!(!ptr.is_null());
        assert_eq!(unsafe { *ptr }, b'1');
        assert_eq!(len, 1);
    }

    #[test]
    fn call_param_to_ptr_two() {
        let (ptr, len) = call_param_to_ptr(b"15");
        assert!(!ptr.is_null());
        assert_eq!(unsafe { *ptr }, b'1');
        assert_eq!(unsafe { *ptr.add(1) }, b'5');
        assert_eq!(len, 2);
    }

    #[test]
    fn call_param_to_ptr_mut_empty() {
        let mut bytes: [u8; 0] = *b"";

        let (ptr, len) = call_param_to_ptr_mut(bytes.as_mut_slice());
        assert!(ptr.is_null());
        assert_eq!(len, 0);
    }

    #[test]
    fn call_param_to_ptr_mut_one() {
        let mut bytes: [u8; 1] = *b"1";

        let (ptr, len) = call_param_to_ptr_mut(bytes.as_mut_slice());
        assert!(!ptr.is_null());
        assert_eq!(unsafe { *ptr }, b'1');
        assert_eq!(len, 1);
    }

    #[test]
    fn call_param_to_ptr_mut_two() {
        let mut bytes: [u8; 2] = *b"15";

        let (ptr, len) = call_param_to_ptr_mut(bytes.as_mut_slice());
        assert!(!ptr.is_null());
        assert_eq!(unsafe { *ptr }, b'1');
        assert_eq!(unsafe { *ptr.add(1) }, b'5');
        assert_eq!(len, 2);
    }

    #[test]
    fn call_param_to_slice_empty() {
        let slice = unsafe { call_param_to_slice::<u8>(ptr::null(), 0) };
        assert_eq!(slice.len(), 0);
    }

    #[test]
    #[should_panic]
    fn call_param_to_slice_none_empty_null() {
        unsafe { call_param_to_slice::<u8>(ptr::null(), 1) };
    }

    #[test]
    fn call_param_to_slice_one() {
        let (ptr, len) = call_param_to_ptr(b"1");

        let slice = unsafe { call_param_to_slice(ptr, len) };
        assert_eq!(slice.len(), 1);
        assert_eq!(slice, b"1");
    }

    #[test]
    fn call_param_to_slice_two() {
        let (ptr, len) = call_param_to_ptr(b"15");

        let slice = unsafe { call_param_to_slice(ptr, len) };
        assert_eq!(slice.len(), 2);
        assert_eq!(slice, b"15");
    }

    #[test]
    fn call_param_to_slice_mut_empty() {
        let slice = unsafe { call_param_to_slice_mut::<u8>(ptr::null_mut(), 0) };
        assert_eq!(slice.len(), 0);
    }

    #[test]
    #[should_panic]
    fn call_param_to_slice_mut_none_empty_null() {
        unsafe { call_param_to_slice_mut::<u8>(ptr::null_mut(), 1) };
    }

    #[test]
    fn call_param_to_slice_mut_one() {
        let mut bytes: [u8; 1] = *b"1";

        let (ptr, len) = call_param_to_ptr_mut(bytes.as_mut_slice());

        let slice = unsafe { call_param_to_slice_mut(ptr, len) };
        assert_eq!(slice.len(), 1);
        assert_eq!(slice, b"1");
    }

    #[test]
    fn call_param_to_slice_mut_two() {
        let mut bytes: [u8; 2] = *b"15";

        let (ptr, len) = call_param_to_ptr_mut(bytes.as_mut_slice());

        let slice = unsafe { call_param_to_slice_mut(ptr, len) };
        assert_eq!(slice.len(), 2);
        assert_eq!(slice, b"15");
    }
}

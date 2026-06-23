package com.imsviewer.android.data

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

class SecurePasswordStore(context: Context) {
    private val store = context.getSharedPreferences("secure_passwords", Context.MODE_PRIVATE)

    fun save(recordId: String, password: String): Boolean {
        return runCatching {
            val cipher = Cipher.getInstance(Transformation)
            cipher.init(Cipher.ENCRYPT_MODE, getOrCreateKey())
            val encrypted = cipher.doFinal(password.toByteArray(Charsets.UTF_8))
            store.edit()
                .putString(recordId, "${encode(cipher.iv)}:${encode(encrypted)}")
                .apply()
        }.isSuccess
    }

    fun load(recordId: String): String? {
        val value = store.getString(recordId, null) ?: return null
        val parts = value.split(":")
        if (parts.size != 2) return null
        return runCatching {
            val cipher = Cipher.getInstance(Transformation)
            cipher.init(Cipher.DECRYPT_MODE, getOrCreateKey(), GCMParameterSpec(GcmTagBits, decode(parts[0])))
            String(cipher.doFinal(decode(parts[1])), Charsets.UTF_8)
        }.getOrNull()
    }

    fun has(recordId: String): Boolean = store.contains(recordId)

    fun delete(recordId: String) {
        store.edit().remove(recordId).apply()
    }

    private fun getOrCreateKey(): SecretKey {
        val keyStore = KeyStore.getInstance(AndroidKeyStore).apply { load(null) }
        (keyStore.getKey(KeyAlias, null) as? SecretKey)?.let { return it }
        val keyGenerator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, AndroidKeyStore)
        keyGenerator.init(
            KeyGenParameterSpec.Builder(
                KeyAlias,
                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setRandomizedEncryptionRequired(true)
                .build()
        )
        return keyGenerator.generateKey()
    }

    private fun encode(bytes: ByteArray): String = Base64.encodeToString(bytes, Base64.NO_WRAP)

    private fun decode(value: String): ByteArray = Base64.decode(value, Base64.NO_WRAP)

    private companion object {
        const val AndroidKeyStore = "AndroidKeyStore"
        const val KeyAlias = "imsviewer_sftp_passwords"
        const val Transformation = "AES/GCM/NoPadding"
        const val GcmTagBits = 128
    }
}

package org.p2pshare.test;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import javax.net.ssl.HttpsURLConnection;

final class ReleaseUpdates {
    private static final String API =
        "https://api.github.com/repos/ruthraillu/P2PShare/releases/latest";
    private static final int MAX_METADATA = 256 * 1024;

    private ReleaseUpdates() {}

    static String fetchLatest() throws IOException {
        HttpsURLConnection connection = (HttpsURLConnection) new URL(API).openConnection();
        try {
            connection.setConnectTimeout(8000);
            connection.setReadTimeout(8000);
            connection.setInstanceFollowRedirects(false);
            connection.setRequestProperty("Accept", "application/vnd.github+json");
            connection.setRequestProperty("User-Agent", "P2PShare-Android");
            if (connection.getResponseCode() != 200) {
                return null;
            }
            try (InputStream input = connection.getInputStream()) {
                ByteArrayOutputStream output = new ByteArrayOutputStream();
                byte[] buffer = new byte[8192];
                int count;
                while ((count = input.read(buffer)) != -1) {
                    if (output.size() + count > MAX_METADATA) {
                        throw new IOException("Update metadata exceeded its limit");
                    }
                    output.write(buffer, 0, count);
                }
                return new String(output.toByteArray(), StandardCharsets.UTF_8);
            }
        } finally {
            connection.disconnect();
        }
    }
}
